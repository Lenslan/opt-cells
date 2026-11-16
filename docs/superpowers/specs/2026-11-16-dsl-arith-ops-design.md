# DSL 扩充：算术、大小比较与动态下标 — Design Spec

**Date**: 2026-11-16
**Status**: Implemented

## 1. 目标

DSL 原先只有 1-bit 布尔运算和向量 `==`/`!=`。本次增加：

- 大小比较 `<`、`<=`、`>`、`>=`
- 算术 `+`、`-`、`*`
- 动态下标 `bitmap[idx]`
- 为以上功能所必需的：无位宽十进制整数（`10`）、多 bit 赋值（整体与切片）、多 bit 中间信号、多 bit `?:`、按位 `~`

不在范围内：`&&`、`||`、移位、拼接、缩位运算、有符号数。

## 2. 方案

向量成为前端的一等值，所有新运算在 elaborate 阶段展开为 1-bit AIG。
`aig`（除新增 `bitvec.rs`）、`match_npn`、`mapper`、`report` 不变，映射与 Tcl 输出原样复用。

否决的方案：

- 字级 IR + 算术宏单元（FA 等）：需要多输出 cell，库格式和 mapper 都要重写。
- 源到源改写为旧 DSL：上下文位宽规则和报错位置难以处理。

## 3. 语法

优先级从高到低：`! ~` → `*` → `+ -` → `< <= > >=` → `== !=` → `&` → `^` → `|` → `?:`。
`* + - < <= > >=` 左结合。

- `name[expr]`：`expr` 是单个整数时仍是静态位选择 `BitSel::Index`，否则是 `Expr::DynIndex`。
- 赋值左侧：`name`、`name[i]`、`name[hi:lo]`。
- 无位宽整数必须能放进 32 位，否则解析报错。

AST 变化：`BitLiteral::Single` → `BitLiteral::Unsized(u64)`；新增 `Expr::DynIndex`、`Expr::BitNot`、
`Expr::Binary { op: BinOp }`；`Lvalue.index` → `Lvalue.sel: Option<BitSel>`。
`!` 解析为 `Expr::Not`（逻辑非），`~` 解析为 `Expr::BitNot`。

## 4. 位宽语义

全部无符号，遵循 Verilog。每个表达式有静态类型 `Ty { width, natural, flex }`：

| 字段 | 含义 |
|---|---|
| `width` | Verilog 自定位宽；无位宽整数为 32 |
| `natural` | 无位宽整数只按其数值所需位数计，用于确定中间信号位宽 |
| `flex` | 位宽中有无位宽整数参与；这类表达式适配上下文，不触发位宽检查 |

求值函数 `eval(expr, width)` 返回恰好 `width` 位：

- 上下文决定位宽的运算（`+ - * & ^ | ~`、`?:` 的两个分支）把 `width` 传给操作数。
  这些运算结果的低 N 位只依赖操作数的低 N 位，所以直接用目标位宽求值与 Verilog
  “先扩展到 max(左, 右) 再截断”等价。
- 比较运算的操作数位宽为 `max(左.width, 右.width)`。
- `!x`、`?:` 的条件、下标表达式按自身 `width` 求值。
- 赋值的上下文位宽是左侧位宽。新中间信号的位宽是右侧的 `natural`。

严格检查（`width mismatch`），仅当相关表达式都不是 `flex`：

- 比较两侧 `width` 不相等。
- 赋值右侧 `width` 大于左侧。
- `?:` 两个分支 `width` 不相等；条件不是 1 bit。

动态下标：以声明下标为地址建表（低于 `lo` 的位置填 0），用下标的低 k 位做 MUX 树
（k 为寻址到 `hi` 所需位数），其余高位任一为 1 则结果为 0。越界读出 0。

## 5. 电路结构（`src/aig/bitvec.rs`）

| 函数 | 结构 |
|---|---|
| `add` | 行波进位，每位 `sum = a^b^c`，`carry = a&b \| (a^b)&c` |
| `sub` | `a + ~b + 1` |
| `mul` | 阵列乘法器，第 i 行部分积只加到 i 以上的位 |
| `eq` | 逐位同或再相与 |
| `ult` | 从低位起 `lt' = (!a&b) \| (lt & !(a&!b))` |
| `select` | 二叉 MUX 树 + 越界清零 |
| `mux` | 两臂相同则直接返回，不产生节点 |

常数操作数通过 `Aig::and` 的常量化简自动折叠。

## 6. 行为变化

- 多 bit 赋值由报错变为合法。
- `0`/`1` 成为无位宽整数。1-bit 用法结果不变；`~` 作用于含此类字面量的表达式且结果
  参与比较或作条件时按 32 位计算（与 Verilog 一致）。
- 新报错：同一输出 bit 重复赋值；给 `input` 赋值。
- 单元库 `function` 中拒绝新运算符和 0/1 以外的整数。

## 7. 验证

- `src/aig/bitvec.rs` 单元测试：各构造函数对整数模型穷举。
- `tests/dsl_arith.rs`：每个特性穷举仿真，AIG 与映射后网表都对比整数参考模型；另有报错用例。
- 回归：全部 fixture × 全部库以及 `workspace/ouyanglin.dsl` 共 63 份报告，改动前后逐字节一致。
