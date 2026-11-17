# 自定义名称前缀 `--prefix` — Design Spec

**Date**: 2026-11-17
**Status**: Implemented

## 1. 目标

工具为新建的 cell 和 net 统一加 `eco_` 前缀。实例名和内部 net 名由 uid、节点号生成
（`eco_<CELL>_u0`、`eco_n3_u1` …），所以两次运行的结果落到同一个设计时会重名，也无法从名字
看出属于哪一次。

本次增加命令行参数 `-p, --prefix <PREFIX>`：传入 `test` 后，本次运行新建的全部 cell 和 net
都命名为 `eco_test_xxx`。

不在范围内：去掉或替换 `eco_` 本身；名字冲突检测（与现状一致，不检测）；在报告头部显示前缀。

## 2. 行为

| | 不传（现状） | `--prefix test` |
|---|---|---|
| cell 实例 | `eco_<CELL>_u<uid>` | `eco_test_<CELL>_u<uid>` |
| 默认内部 net | `eco_n<node>_u<uid>` | `eco_test_n<node>_u<uid>` |
| DSL 别名 `eco_xxx` | `eco_xxx` | `eco_test_xxx` |
| `input` / `output` 端口、常量 `0` `1` | 原名 | 原名 |

- 不传 `--prefix` 时输出与改动前逐字节一致。
- 带前缀的输出等于把不带前缀输出中每个新建名字开头的 `eco_` 换成 `eco_<PREFIX>_`，
  网表结构、cell 数、uid 都不变。这个替换在新建名字之间一一对应，不会让原本不同的两个
  新建名字变成同名。
- “新建”指脚本里由 `create_cell` / `create_net` 创建的名字。`input` / `output` 声明的名字
  即使以 `eco_` 开头也保持原样：端口是脚本与已有 net 之间的接口，那个 net 可能由另一份
  ECO 脚本以别的前缀创建，改名会让引用落空。
- DSL 不需要修改：中间信号仍以 `eco_` 开头来表示“保留这个名字”，输出时这个 `eco_` 被换成
  完整前缀。多 bit 别名的每一位为 `eco_<PREFIX>_xxx[i]`。DSL 中已经写成 `eco_test_ab` 的名字
  不做去重，输出为 `eco_test_test_ab`。
- 报告的 Mapped netlist 段：别名与 Tcl 段保持同一个名字（`eco_test_xxx`）；`u<uid>`、
  `n<node>_u<uid>` 这类简写标签本来就不带 `eco_`，不变。
- `--quiet` 只输出 cell 数，前缀对结果没有影响（非法前缀仍然报错）。

### 校验

`PREFIX` 必须非空，且只包含 ASCII 字母、数字和 `_`。否则报错 `invalid prefix '<值>': ...`，
退出码为 1，不产生报告。

- 限制字符集的理由：名字会原样写进 `create_cell` / `create_net` / `[get_pin inst/pin]`，
  `/`、`[`、空格、`*` 等字符会改变 Tcl 或层次路径的含义。
- 空串按错误处理而不是退回 `eco_`：脚本里变量为空时如果静默退回默认前缀，生成的名字会和
  上一批 ECO 冲突。

## 3. 方案

命名规则全部放在 `src/report/mod.rs`，前端和 mapper 不感知前缀。

| 文件 | 变化 |
|---|---|
| `src/cli.rs` | `Args` 增加 `prefix: Option<String>`（`-p, --prefix`） |
| `src/main.rs` | 把 `args.prefix` 传入 `RunInputs` |
| `src/lib.rs` | `RunInputs` 增加 `prefix: Option<String>`；`run_pipeline` 开头调用 `report::name_prefix` 校验并得到完整前缀 |
| `src/report/mod.rs` | `name_prefix(Option<&str>) -> Result<String, OptCellsError>` 返回 `eco_` 或 `eco_<PREFIX>_`；`ReportInput` 增加 `name_prefix: &str`；实例名、内部 net 名、别名都由它生成 |
| `src/error.rs` | 增加 `OptCellsError::InvalidPrefix` |

`RunInputs` 和 `ReportInput` 是公开结构体，新增字段后现有调用点需要补 `prefix: None` /
`name_prefix: "eco_"`。

别名在 report 中只有一个出口 `alias_label`：别名开头的 `eco_` 换成 `name_prefix`。直接构造
`ReportInput` 时传入的、不以 `eco_` 开头的别名是调用者给定的名字，保持原样（经过
`run_pipeline` 不会出现这种别名，前端只记录 `eco_` 开头的中间信号）。

否决的方案：

- 对渲染好的文本做 `eco_` → `eco_test_` 字符串替换：名字里带 `eco_` 的端口会被误改。
- 在 `elaborate` 记录别名时加前缀：别名规则在前端、实例名和内部 net 规则在 report，
  同一条规则分散在两个模块。

## 4. 验证

- `src/report/mod.rs` 单元测试：`name_prefix` 的默认值、合法与非法输入；不带 `eco_` 的别名
  在带前缀渲染时保持原样。
- `tests/name_prefix.rs`：
  - 带前缀时的完整 Tcl 命令序列（实例、内部 net、别名、连线）；别名在 Mapped netlist 段
    同步改名；多 bit 别名逐位改名。
  - 名字以 `eco_` 开头的标量和向量端口不被改名。
  - 9 组 fixture × 库以及 4 个较大的内联用例：带前缀的报告中不再有裸的 `eco_` 名字，
    把 `eco_<PREFIX>_` 换回 `eco_` 后与默认报告逐 token 相同。
  - 非法前缀返回 `InvalidPrefix`。
  - 通过真实可执行文件验证 `--prefix` / `-p` 生效、`--quiet` 不受影响、非法前缀的退出码为 1。
- 回归：全部 fixture 输入和 `workspace/` 下的 DSL × 全部库共 91 次运行（60 份报告、31 次报错），
  不传前缀时 stdout、stderr 和退出码与改动前逐字节一致；现有 golden 测试不变。
