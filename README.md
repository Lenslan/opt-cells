# opt-cells

> 将组合逻辑表达式映射到**单元数最少**的标准单元库实现的 Rust 命令行工具。

`opt-cells` 接收两份输入——用自定义 DSL 写的组合逻辑、以及用 TOML 描述的单元库（cell library）——
输出把逻辑映射到库中单元后**单元数量最少**的实现方案，并打印一份可读的文本报告。

它的定位是「工业级综合器的一个子集」：比教学玩具更实用，但明确比 ABC / Yosys / Synopsys DC 窄得多。

---

## 目录

- [它解决什么问题](#它解决什么问题)
- [工作原理](#工作原理)
- [项目结构](#项目结构)
- [构建与安装](#构建与安装)
- [快速开始](#快速开始)
- [命令行用法](#命令行用法)
- [DSL 语法](#dsl-语法)
- [单元库格式](#单元库格式)
- [输出报告解读](#输出报告解读)
- [错误诊断](#错误诊断)
- [算法细节](#算法细节)
- [测试](#测试)
- [作用范围与非目标](#作用范围与非目标)
- [依赖与许可证](#依赖与许可证)

---

## 它解决什么问题

技术映射（technology mapping）的核心难题之一：同一个逻辑功能，在不同的输入取反 / 输入排列 / 输出取反下
其实是「同一类」单元。手工把所有变体都写进库里既繁琐又容易出错。

`opt-cells` 用 **NPN 等价匹配** 自动处理这件事，于是：

- 输入 `y = !(a & b);`，若库里有 `NAND2`，得到 **1 个单元** 的映射（而不是 `AND2` + `INV` 两个）。
- 输入 `decoded = (state[3:0] == 4'b0111);`，若库里有 `AND4`，得到 **1 个 AND4**（最高位输入取反），
  而不是 4 个以上的单元。

你**不需要**在库里枚举各种「输入取反」的变体——NPN 等价匹配会自动发现这些等价关系。

> 一个值得注意的细节：`!(a & b)` 按德摩根律等于 `!a | !b`，所以「两个输入都取反的 OR2」与 `NAND2`
> 属于**同一个 NPN 类**。下面快速开始里你会看到，给定一个同时含 `OR2` 和 `NAND2` 的库时，
> 工具找到的就是一个单元的解（具体落在 `OR2` 还是 `NAND2` 只是同代价候选里的实现细节，二者功能完全等价）。

---

## 工作原理

整条流水线是**单进程、纯函数式、无共享状态**的，便于测试，也便于将来替换某一层。

```
input.dsl ──parse──► AST ──elaborate──► AIG
                                         │
library.toml ──load──► CellLib ──npn-index──► NpnLibIndex
                                         │              │
                                         ▼              ▼
                                      k-cut 枚举       查表
                                         │              │
                                         └──────► Mapper(两阶段 DP)
                                                      │
                                                      ▼
                                                 MappedNetlist
                                                      │
                                                      ▼
                                                   文本报告
```

各阶段：

1. **解析（frontend::parser）**：用 `chumsky` 把 DSL 文本解析成带源码区间（span）的 AST。
2. **细化（frontend::elaborate）**：把位向量拆成单比特信号（`state[3:0]` → 内部 4 个独立输入），
   把 `|`/`^`/`==` 等运算下降成只含 AND 与取反的形式，构建 **AIG**（And-Inverter Graph）。
   AIG 用 hash-consing 自动去重——多个输出共享的子表达式只构建一次。
3. **加载库（frontend::library）**：解析 TOML，用同一个 DSL 解析器解析每个单元的 `function`，
   为每个单元算出真值表（≤6 输入，存进一个 `u64`）。
4. **建立 NPN 索引（match_npn）**：对每个单元的真值表求 NPN 规范型，建成
   `HashMap<(规范真值表, 输入数), Vec<InputMapping>>`。
5. **枚举 k-cut（aig::cuts）**：对 AIG 每个节点枚举叶子数 ≤6 的子图（cut），并算出每个 cut 的真值表。
6. **覆盖映射（mapper）**：对每个 cut 求 NPN 规范型，到索引里查可用单元，再用**两阶段动态规划**
   选出单元数最少的覆盖方案。
7. **报告（report）**：把映射结果格式化成「摘要 + 网表 + 单元用量」三段式文本。

### 关键的代价模型假设：输入引脚取反是「免费」的

当 NPN 匹配判定某个单元可以实现一个 cut、只是某些输入需要以取反形式到达时，
工具把它算作**单个单元**的映射——报告里用 `pin=!signal` 标注这个取反，**不会**额外计一个 INV 单元。

依据：真实标准单元库通常本就提供「带内建反相输入引脚」的单元。例如 TSMC 的
`INR4D0BWP7T40P140` 原生功能是 `out = !(!A1 | B1 | B2 | B3)`，引脚 `A1` 上的取反是单元物理定义的一部分。

只有在**输出极性需要显式调和**时才会真正计入 INV 单元——典型场景见
[快速开始](#示例-3只有-and2--inv-时何时会计入-inv) 中「库里只有 AND2 + INV」的例子。

---

## 项目结构

单 crate（`opt-cells`），所有非 CLI 逻辑都在 `lib.rs` 的各模块里，`main.rs` 只做命令行接线。

```
opt-cells/
├── Cargo.toml                  # 包定义、依赖、bin/lib 目标
├── README.md                   # 本文件
├── src/
│   ├── main.rs                 # CLI 入口：读输入、调用 run_pipeline、渲染错误、决定退出码
│   ├── lib.rs                  # 库根：导出各模块 + run_pipeline 一站式流水线函数
│   ├── cli.rs                  # clap 参数定义（Args）
│   ├── error.rs                # Span + OptCellsError 错误类型 + ariadne 渲染
│   ├── frontend/               # 前端：DSL → AST → AIG，以及库加载
│   │   ├── ast.rs              #   AST 节点定义（Program / Decl / Expr / ...）
│   │   ├── parser.rs           #   chumsky 解析器（表达式 / 声明 / 语句 / 程序）
│   │   ├── elaborate.rs        #   语义细化：向量展开、运算下降、构建 AIG、出错诊断
│   │   └── library.rs          #   加载 TOML 单元库，计算每个单元的真值表
│   ├── aig/                    # And-Inverter Graph 中间表示
│   │   ├── node.rs             #   NodeId / Edge / NodeKind(Const0|PI|And2) / AigNode
│   │   ├── builder.rs          #   Aig 构建器：hash-consing、and/or/xor、常量化简
│   │   ├── truth_table.rs      #   Tt64：≤6 输入真值表（u64）及位运算
│   │   └── cuts.rs             #   k-cut 枚举（k=6）、cut 支配剪枝
│   ├── match_npn/              # NPN 规范型与库索引
│   │   ├── canonical.rs        #   NPN 规范型暴力求解 + 变换应用 apply_transform
│   │   └── index.rs            #   NpnLibIndex：建库索引 + cut→单元映射 matches()
│   ├── mapper/                 # 两阶段 DP 覆盖
│   │   ├── netlist.rs          #   MappedNetlist / CellInstance / PinInput 数据结构
│   │   ├── phase1.rs           #   自底向上估算每节点正/负极性最小代价
│   │   └── phase2.rs           #   自顶向下提交，生成最终单元实例
│   └── report/
│       └── mod.rs              #   渲染三段式文本报告
├── tests/
│   ├── golden.rs               # 端到端黄金测试（DSL × 库 → 对比 expected/*.txt）
│   ├── proptest.rs             # 属性测试：NPN 幂等、功能等价、单调性
│   ├── common/mod.rs           # 仅测试用的 AIG/网表仿真器
│   └── fixtures/
│       ├── inputs/*.dsl        # 示例输入：nand / decode / mux / shared
│       ├── libs/*.toml         # 示例库：basic / with_and4 / and2_only
│       └── expected/*.txt      # 对应的黄金输出
├── docs/superpowers/           # 设计文档
│   ├── specs/...-design.md     #   完整设计规格（强烈推荐阅读）
│   └── plans/...-opt-cells.md  #   实现计划
└── .github/workflows/ci.yml    # CI：cargo fmt --check / clippy -D warnings / test
```

---

## 构建与安装

需要 Rust 工具链（MSRV = **1.75**，edition 2021）。

```bash
# 构建（调试版）
cargo build

# 构建后二进制位于：
#   Linux/macOS : target/debug/opt-cells
#   Windows     : target/debug/opt-cells.exe

# 发布版（更快）
cargo build --release      # → target/release/opt-cells

# 从本地源码安装到 ~/.cargo/bin
cargo install --path .
```

无需先 `cargo build` 也可以直接用 `cargo run` 运行（见下）。

---

## 快速开始

> 下面命令用仓库自带的 `tests/fixtures/` 作为示例输入。
> 凡是写 `opt-cells ...` 的地方，都等价于 `cargo run --quiet -- ...`。

### 示例 1：NAND（NPN 等价的威力）

```bash
opt-cells -l tests/fixtures/libs/basic.toml tests/fixtures/inputs/nand.dsl
```

输入 `nand.dsl` 是 `y = !(a & b);`。`basic.toml` 含 `INV/AND2/OR2/NAND2/NOR2`。输出：

```
═══════════════════════════════════════════════════════════════
  opt-cells mapping report
═══════════════════════════════════════════════════════════════
  Input file       : tests/fixtures/inputs/nand.dsl
  Cell library     : tests/fixtures/libs/basic.toml
  Total cells used : 1
───────────────────────────────────────────────────────────────
  Mapped netlist
───────────────────────────────────────────────────────────────
  u0 : OR2  (a=!a, b=!b)  -> y

───────────────────────────────────────────────────────────────
  Cell usage
───────────────────────────────────────────────────────────────
  OR2 × 1
═══════════════════════════════════════════════════════════════
```

只用了 **1 个单元**：`!(a & b)` ≡ `!a | !b`，于是「两输入取反的 OR2」就是答案
（`NAND2` 同样是 1 个单元的合法解，二者属于同一 NPN 类、功能完全等价）。

### 示例 2：4 位译码（自动反相最高位输入）

```bash
opt-cells -l tests/fixtures/libs/with_and4.toml tests/fixtures/inputs/decode.dsl
```

输入 `decode.dsl` 是 `decoded = (state == 4'b0111);`。输出：

```
═══════════════════════════════════════════════════════════════
  opt-cells mapping report
═══════════════════════════════════════════════════════════════
  Input file       : tests/fixtures/inputs/decode.dsl
  Cell library     : tests/fixtures/libs/with_and4.toml
  Total cells used : 1
───────────────────────────────────────────────────────────────
  Mapped netlist
───────────────────────────────────────────────────────────────
  u0 : AND4  (a=state[0], b=state[1], c=state[2], d=!state[3])  -> decoded

───────────────────────────────────────────────────────────────
  Cell usage
───────────────────────────────────────────────────────────────
  AND4 × 1
═══════════════════════════════════════════════════════════════
```

`state == 4'b0111` 即 `state[3]==0 && state[2..0]==1`，映射成 **1 个 AND4**，最高位 `state[3]` 自动取反
（`d=!state[3]`），无需额外的反相器。

### 示例 3：只有 AND2 + INV 时——何时会计入 INV

```bash
opt-cells -l tests/fixtures/libs/and2_only.toml tests/fixtures/inputs/nand.dsl
```

同样的 `y = !(a & b);`，但库里只有 `AND2` 和 `INV`（没有任何 NAND 类单元），输出：

```
═══════════════════════════════════════════════════════════════
  opt-cells mapping report
═══════════════════════════════════════════════════════════════
  Input file       : tests/fixtures/inputs/nand.dsl
  Cell library     : tests/fixtures/libs/and2_only.toml
  Total cells used : 2
───────────────────────────────────────────────────────────────
  Mapped netlist
───────────────────────────────────────────────────────────────
  u0 : INV  (a=n3_u1)  -> y
  u1 : AND2  (a=a, b=b)  -> !y

───────────────────────────────────────────────────────────────
  Cell usage
───────────────────────────────────────────────────────────────
  AND2 × 1
  INV × 1
═══════════════════════════════════════════════════════════════
```

读法：`u1` 的 AND2 计算 `a & b`（即 `!y`），`u0` 的 INV 把它取反得到 `y`。
因为库里没有 NAND 类单元，输出极性必须显式调和，所以这里确实计入了一个 INV——共 **2 个单元**。

### 示例 4：从标准输入读取 + 只看单元数

```bash
# 通过管道喂 DSL（INPUT 用 "-" 表示 stdin）
echo 'input a, b; output y; y = !(a & b);' | opt-cells -l tests/fixtures/libs/basic.toml -

# -q / --quiet：只打印一行单元总数，方便批处理脚本
opt-cells -q -l tests/fixtures/libs/basic.toml tests/fixtures/inputs/mux.dsl
# 输出： total cells: 3
```

---

## 命令行用法

```
opt-cells [OPTIONS] --library <LIBRARY> <INPUT>

参数:
  <INPUT>                  DSL 输入文件路径，或用 "-" 从标准输入读取

选项:
  -l, --library <LIBRARY>  单元库 TOML 文件        [必填]
  -o, --output <OUTPUT>    把报告写入文件（默认写到 stdout）
  -q, --quiet              只打印 "total cells: N" 一行摘要
  -h, --help               打印帮助
  -V, --version            打印版本
```

退出码：

| 码 | 含义 |
|---|---|
| `0`   | 成功 |
| `1`   | 用户错误（DSL/库 语法、语义、映射不可行） |
| `2`   | 命令行参数错误（clap 默认） |
| `101` | panic（内部 bug，rustc 默认） |

---

## DSL 语法

### 文法（EBNF）

```ebnf
program     := decl* statement+

decl        := "input"  name_list ";"
             | "output" name_list ";"
name_list   := ident width? ("," ident width?)*
width       := "[" int ":" int "]"            ; 例如 [3:0]

statement   := lvalue "=" expr ";"
lvalue      := ident ("[" int "]")?           ; 左值只能单比特下标（不能写范围）

expr        := or_expr
or_expr     := xor_expr ("|" xor_expr)*
xor_expr    := and_expr ("^" and_expr)*
and_expr    := eq_expr  ("&" eq_expr)*
eq_expr     := unary    (("==" | "!=") unary)?
unary       := ("!" | "~") unary | primary
primary     := "(" expr ")" | literal | signal_ref
signal_ref  := ident ("[" int (":" int)? "]")?
literal     := int "'" base digits            ; 例如 4'b0111, 8'hFF, 4'd7
             | "0" | "1"                       ; 单比特字面量
```

### 运算符与优先级

从**高到低**（注意：相等比较 `==`/`!=` 比 `&` 结合得更紧）：

| 优先级 | 运算符 | 含义 | 结合性 |
|---|---|---|---|
| 1（最高） | `!` `~`   | 逻辑取反 | 右结合（一元前缀） |
| 2 | `==` `!=` | 相等 / 不等比较 | 非结合（最多一次） |
| 3 | `&`       | 与 | 左结合 |
| 4 | `^`       | 异或 | 左结合 |
| 5（最低） | `\|`     | 或 | 左结合 |

用括号 `( ... )` 可覆盖默认优先级。

### 字面量

- 单比特：`0`、`1`
- 位向量：`<位宽>'<进制><数字>`，进制为 `b`(二进制) / `h`(十六进制) / `d`(十进制)
  - 例：`4'b0111`、`8'hFF`、`4'd7`
- 位向量字面量只能出现在 `==` / `!=` 比较里（标量上下文中使用会报错）。

### 语义要点

- 单比特 `a == b` 下降为 `!(a ^ b)`。
- 向量 `==` 下降为「逐位相等」的按位与。
- **向量在细化阶段被拆成单比特信号**：声明 `input state[3:0];` 会内部生成 4 个独立的主输入，
  报告里再还原成 `state[3]`、`state[2]` 这样的显示名。
- 向量引用**只能**出现在相等比较中；把整个向量用在标量布尔运算里会报错。
- 多条 `output = expr;` 之间共享的子表达式由 AIG 的 hash-consing 自动去重（见 `shared.dsl`）。

### 限制（会给出明确报错）

- 多比特整体赋值 `y[3:0] = expr;`：不支持，需逐位写（`y[0] = ...; y[1] = ...;`）。
  但对已声明为向量的输出，**逐位下标赋值** `y[0] = ...;` 是允许的。
- 表达式中位宽不匹配，例如 `y = (state == 3'b011)` 而 `state` 是 4 位。
- 引用未声明的信号。
- 对单比特信号使用下标，或下标越界。

### DSL **不**包含（有意为之）

`always` / `if` / `case` / `wire`；算术（`+ - *`）；移位（`<< >>`）；
有符号/无符号区分；模块层次。

### 一个完整的 DSL 例子

```
input  a, b;
input  state[3:0];
output y;
output decoded;

y       = !(a & b);
decoded = (state == 4'b0111);
```

---

## 单元库格式

单元库是 TOML 文件，由若干 `[[cell]]` 表组成：

```toml
[[cell]]
name = "INV"
inputs = ["a"]
output = "y"
function = "!a"

[[cell]]
name = "NAND2"
inputs = ["a", "b"]
output = "y"
function = "!(a & b)"

[[cell]]
name = "AND4"
inputs = ["a", "b", "c", "d"]
output = "y"
function = "a & b & c & d"

[[cell]]
name = "AOI21"
inputs = ["a", "b", "c"]
output = "y"
function = "!((a & b) | c)"
```

字段：

| 字段 | 说明 |
|---|---|
| `name`     | 单元名（库内唯一） |
| `inputs`   | 输入引脚名列表，顺序即引脚顺序 |
| `output`   | 输出引脚名 |
| `function` | 单元功能，**复用 DSL 表达式语法**，只能引用 `inputs` 中列出的变量 |

设计要点：

- **`function` 复用 DSL 语法**——同一个解析器同时处理输入文件和库文件。
- **无需枚举变体**——NPN 匹配自动覆盖输入取反/排列，库里只写基本形即可。
- **没有面积/延迟字段**——只优化单元数用不到，留作将来扩展。
- **输入数限制 [1, 6]**——超出此范围的单元会加载成功但**无法被匹配**（cut 枚举上限 k=6），
  加载时打印一行提示信息。

校验规则：

- 单元名重复 → 报错。
- `function` 引用了不在 `inputs` 里的变量 → 报错。
- 输入数为 0 或 >6 → 打印一行提示（单元仍加载，但不可匹配），不算错误。
- 空库（没有任何单元）→ 报错。
- 库里没有 INV 类单元、而映射又确实需要反相 → 在映射阶段针对具体信号报错。

> **如何判定 INV 单元？** 工具识别「单输入、真值表为 `!a`」的单元作为 INV
> （用于输出极性调和），无论它叫什么名字。

---

## 输出报告解读

默认输出三段式报告：

```
  u0 : OR2   (a=n4_u1, b=n5_u2)  -> y
  └┬┘  └─┬┘  └────────┬───────┘    └┬┘
 实例号  单元名   各引脚=驱动信号        输出目标
```

- **实例号** `u0, u1, ...`：按生成顺序编号的单元实例。
- **引脚连接** `(pinName=driver, ...)`：左侧是库里声明的引脚名，右侧是驱动该引脚的信号：
  - 主输入直接显示其（还原后的）名字，如 `a`、`state[3]`。
  - 内部 AND 节点显示为 `n<节点号>_u<实例号>`，如 `n4_u1` 表示由实例 `u1` 驱动的 AIG 节点 4。
  - 取反的输入显示为 `pin=!driver`（单元吸收了这个取反，**不**额外计 INV）。
- **输出目标** `-> ...`：
  - 若该节点驱动某个主输出，显示主输出名（如 `y`、`decoded`）；必要时带极性前缀 `!`。
  - 否则显示内部名 `n<节点号>`。
- **Cell usage**：按单元名字典序统计各类单元的用量（`NAME × N`）。

---

## 错误诊断

DSL 的解析错误和细化错误会用 `ariadne` 渲染，带源码区间高亮。例如对

```
input a;
output y;
y = a & b;     # b 未声明
```

会得到：

```
Error: undefined signal 'b'
   ╭─[<stdin>:3:9]
   │
 3 │ y = a & b;
   │         ┬
   │         ╰── undefined signal 'b'
───╯
```

库错误和映射错误以普通文本打印（库文件不跟踪字节级位置）。常见诊断场景：

| 场景 | 诊断信息（大意） |
|---|---|
| 引用未声明信号 | `undefined signal 'X'` |
| 向量位宽不匹配 | `width mismatch: lhs is 4 bits, rhs is 3 bits` |
| 多比特整体赋值 | `multi-bit assignment not supported; write one assignment per bit` |
| 单元名重复 | `duplicate cell name 'X'` |
| `function` 引用未知变量 | `function references 'd' which is not in inputs list` |
| 需要反相但库无 INV | `node N requires inversion but library has no INV-class cell` |
| 某节点无任何单元可实现 | `no library cell can implement function at AIG node N` |

诊断只**建议**、绝不修改用户文件。

---

## 算法细节

### AIG（And-Inverter Graph）

只有两种节点：**主输入（PI）** 和 **二输入与（AND2）**；外加一个 `Const0` 常量节点。
每条边带一个取反位（`Edge { node, invert }`），常量 1 = 取反的 Const0 边。

- **Hash-consing**：构建 AND2 时先把两个孩子按 NodeId 归一化（小的在左），再查缓存命中复用。
- **常量化简**：`0 & x = 0`、`1 & x = x`、`x & x = x`、`x & !x = 0`。
- **运算下降**：
  - `a | b` → `!(!a & !b)`
  - `a ^ b` → `!(!(a & !b) & !(!a & b))`
  - `a == b`（单比特）→ `!(a ^ b)`；向量 `==` → 各位 `==` 的与。

### k-cut 枚举（k = 6）

对每个节点 `n` 求 `cuts(n)` = 以 `n` 为根、叶子数 ≤6 的子图：

```
PI:        cuts(PI) = { {PI} }
AND2(l,r): cuts(n)  = { {n} } ∪ { cl ∪ cr | cl∈cuts(l), cr∈cuts(r), |cl∪cr| ≤ 6 }
```

剪枝：每节点最多保留 `CUTS_PER_NODE = 8` 个 cut；丢弃被支配的 cut（叶集为子集者）；
始终保留平凡 cut 以保证可行性。复杂度近似每节点 O(N²)，线性遍历——千节点级 AIG 毫秒量级。

### NPN 规范型

对每个 cut 算真值表（k≤6，存 `u64`），再规约到 NPN 规范型：枚举所有
（输入取反 × 输入排列 × 输出取反）组合（k=6 时最多 2⁶ × 6! × 2 = 92160 种），
取变换后**字典序最小**的真值表为规范型。库索引即
`HashMap<(规范真值表, 输入数), Vec<InputMapping>>`，cut 匹配 = 求 cut 的规范型 → 查表。

### 两阶段 DP 覆盖

带 DAG 共享的最小单元数覆盖是 NP-hard 的，这里用标准的两阶段启发式：

- **阶段 1（自底向上估算）**：拓扑序遍历，对每个节点分别记录产生**正极性**和**负极性**的最小代价、
  对应的 cut 与映射。`mapping_cost = 1 + Σ 各叶子按所需极性的代价`；负极性还可由
  「正极性 + 一个 INV」得到，取两者更小者。PI 的两种极性代价都为 0。
- **阶段 2（自顶向下提交）**：从主输出按所需极性出发，沿阶段 1 选定的 cut 向下提交单元实例，
  把叶子按其所需极性入队，直到队列清空。最终单元数 = 提交的单元实例数（含为极性调和插入的 INV）。

**已知次优性**：阶段 1 估算时假设每个叶子都为当前 cut 单独实现，未计入跨父 cut 的共享，
因此真实代价可能更低；阶段 2 仍沿用阶段 1 的排名而不重新估算。v1 可接受，将来可加一轮精化迭代。

---

## 测试

```bash
# 全部测试（单元 + 集成 + 属性）
cargo test

# 仅黄金端到端测试
cargo test --test golden

# 仅属性测试
cargo test --test proptest

# 重新生成黄金输出（确认改动是预期的之后）
UPDATE_GOLDEN=1 cargo test --test golden
```

测试覆盖：

- **单元测试**：分布在各模块内（解析、细化、AIG hash-consing、cut 枚举、NPN 规范型不变量、phase1/phase2 等）。
- **黄金集成测试**（`tests/golden.rs`）：`DSL × 库` 端到端跑通后比对 `tests/fixtures/expected/*.txt`。
- **属性测试**（`tests/proptest.rs`）：
  - **NPN 幂等**：`canonical(canonical(T)) == canonical(T)`；
  - **功能等价**：随机小 AIG 映射后，在所有输入组合上仿真结果与原 AIG 一致；
  - **单调性**：往库里加单元绝不会让最优单元数变大。
- **仅测试用的仿真器**（`tests/common/mod.rs`）：给定库与输入赋值，按拓扑序仿真 AIG / 网表。

CI（`.github/workflows/ci.yml`）会跑：`cargo fmt --all --check`、
`cargo clippy --all-targets -- -D warnings`、`cargo test --all-targets`。

---

## 作用范围与非目标

**在范围内**：组合逻辑 DSL（布尔运算、位下标、位向量字面量、相等比较）、自定义 TOML 单元库、
AIG 中间表示、k-cut 枚举 + NPN 匹配 + 两阶段 DP 覆盖、可读文本报告、强诊断、完整测试。

**明确不做（v1 的 YAGNI）**：时序逻辑/寄存器/触发器；Verilog 解析（用自定义 DSL）；
Liberty(.lib) 解析（用自定义 TOML）；面积/延迟优化（只优化单元数）；多比特整体赋值；
JSON/Verilog 网表输出（只出文本报告）；模块层次；基于 ILP 的最优覆盖（用 DP 启发式）；
告警分级系统（每个问题要么是错误，要么是一行提示信息）。

---

## 依赖与许可证

| Crate | 用途 |
|---|---|
| `clap`（derive） | 命令行解析 |
| `serde` + `toml` | 单元库反序列化 |
| `chumsky` | DSL 解析器（错误信息友好） |
| `ariadne` | 带源码区间的诊断渲染 |
| `thiserror` | 错误类型派生 |
| `anyhow` | CLI 边界错误类型 |
| `proptest`（dev） | 属性测试 |
| `tempfile`（dev） | 测试临时文件 |

许可证：`MIT OR Apache-2.0`。

更完整的设计背景见 `docs/superpowers/specs/2026-05-21-opt-cells-design.md`。
