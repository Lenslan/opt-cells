# opt-cells

`opt-cells` 是一个 Rust 实现的组合逻辑映射工具。它读取自定义 DSL 描述的布尔逻辑和 TOML 格式的单元库，将逻辑映射为以最少 cell 数为目标的单元级实现，并输出文本报告、ECO Tcl 脚本和 cell usage 统计。

项目定位是一个轻量但工程化的逻辑综合子集：它不解析 Verilog/Liberty，也不做时序、面积或延迟优化，当前优化目标只关注 cell 数。

## 主要功能

- 支持命令行工具 `opt-cells`，也提供可复用的 Rust library pipeline。
- 支持自定义 DSL 描述组合逻辑，包括标量/向量输入输出、位选择、向量比较、中间信号和三目选择。
- 支持 TOML 单元库，cell 的逻辑函数复用同一套表达式语法。
- 基于 AIG、k-cut 枚举、NPN canonical matching 和两阶段 DP covering 进行 cell 映射。
- 严格区分“cell 内建反相”和“额外插入反相器”：额外反相必须由真实 `INV` cell 实现并计入 cell 数。
- 输出 human-readable mapping report，包含 mapped netlist、ECO Tcl script 和 cell usage。
- 带有 unit tests、golden tests、property tests 和 GitHub Actions CI。

## 项目结构

```text
src/
  cli.rs                 # clap 命令行参数定义
  main.rs                # CLI 入口：读文件/stdin、运行 pipeline、写 stdout/文件
  lib.rs                 # run_pipeline，对外复用入口
  error.rs               # 统一错误类型和 ariadne 诊断渲染
  frontend/
    parser.rs            # DSL 表达式、声明和语句解析
    ast.rs               # DSL AST
    elaborate.rs         # DSL AST 展开为 AIG
    library.rs           # TOML 单元库加载和 truth table 计算
  aig/
    builder.rs           # AIG 构建、hash-consing 和基本布尔 lowering
    cuts.rs              # k-cut 枚举
    truth_table.rs       # truth table 运算
  match_npn/
    canonical.rs         # NPN canonical form
    index.rs             # 单元库 NPN 匹配索引
  mapper/
    phase1.rs            # bottom-up cost estimation
    phase2.rs            # top-down commit 生成 mapped netlist
    netlist.rs           # 映射后 netlist 数据结构
  report/
    mod.rs               # 文本报告和 Tcl 渲染

tests/
  fixtures/              # DSL、library、golden report、参考 Tcl
  *.rs                   # integration/property/behavior tests

workspace/
  STATE_3.dsl            # 示例输入
  lib/basic.toml         # 示例标准单元风格库
```

## 构建与运行

### 环境要求

- Rust 1.75 或更新版本
- Cargo

项目使用 Rust 2021 edition。依赖包括 `clap`、`serde`、`toml`、`chumsky`、`ariadne`、`thiserror` 等。

### 构建

```powershell
cargo build --release
```

### 查看帮助

```powershell
cargo run -- --help
```

CLI 参数由 `clap` 生成，核心参数如下：

| 参数 | 说明 |
|---|---|
| `<INPUT>` | DSL 输入文件路径；传 `-` 时从 stdin 读取 |
| `-l, --library <FILE>` | TOML 单元库路径，默认是 `./lib/basic.toml` |
| `-o, --output <FILE>` | 将报告写入文件，默认输出到 stdout |
| `-q, --quiet` | 只输出总 cell 数摘要 |
| `-h, --help` | 显示帮助 |
| `-V, --version` | 显示版本 |

仓库中没有 `./lib/basic.toml`，示例运行时建议显式传入 `-l`。

## 快速开始

### 1. NAND 示例

输入 DSL：

```text
input a, b;
output y;
y = !(a & b);
```

运行：

```powershell
cargo run -- tests/fixtures/inputs/nand.dsl -l tests/fixtures/libs/basic.toml
```

输出报告会包含：

```text
Total cells used : 1

Mapped netlist
---------------------------------------------------------------
  u0 : NAND2  (a=a, b=b)  -> y

Tcl script
---------------------------------------------------------------
create_cell     eco_NAND2_u0                           [get_lib_cells */NAND2]

connect_net     a                                      [get_pin eco_NAND2_u0/a]
connect_net     b                                      [get_pin eco_NAND2_u0/b]
connect_net     y                                      [get_pin eco_NAND2_u0/y]

Cell usage
---------------------------------------------------------------
  NAND2 x 1
```

因为库中有 `NAND2`，工具会使用一个 native NAND cell，而不是 `AND2 + INV`。

### 2. 输出到文件

```powershell
cargo run -- tests/fixtures/inputs/nand.dsl `
  -l tests/fixtures/libs/basic.toml `
  -o report.txt
```

### 3. 只输出 cell 数

```powershell
cargo run -- tests/fixtures/inputs/nand.dsl `
  -l tests/fixtures/libs/basic.toml `
  --quiet
```

输出：

```text
total cells: 1
```

### 4. 从 stdin 读取

```powershell
"input a, b; output y; y = !(a & b);" | cargo run -- - -l tests/fixtures/libs/basic.toml
```

### 5. 使用 workspace 示例库

```powershell
cargo run -- workspace/STATE_3.dsl -l workspace/lib/basic.toml
```

`workspace/lib/basic.toml` 包含一组标准单元风格命名的示例 cell，例如 `INVD0BWP7T40P140`、`AN2D0BWP7T40P140`、`MUX2D0BWP7T40P140`、`XOR4D0BWP7T40P140` 等。

## DSL 语法

DSL 用来描述组合逻辑。所有声明必须出现在赋值语句之前。

### 声明

```text
input a, b, c;
input state[5:0];

output y;
output decoded;
output out_bus[3:0];
```

说明：

- `input` 和 `output` 支持多个逗号分隔的名字。
- 位宽写法是 `[hi:lo]`，内部会展开为单 bit 信号。
- 向量输出需要逐 bit 赋值，当前不支持一次性多 bit 赋值。

### 赋值

```text
y = !(a & b);
decoded = (state == 6'b101011);
out_bus[0] = a ^ b;
out_bus[1] = sel ? a : b;
```

赋值目标可以是：

- 已声明的标量输出，例如 `y`
- 已声明向量输出的某一位，例如 `out_bus[0]`
- 未声明的标量中间信号，例如 `tmp = a & b;`

未声明的标量赋值会被当作中间信号。普通中间信号名主要用于后续表达式引用，不会默认出现在报告/Tcl 中；如果中间信号名以 `eco_` 开头，它会作为内部 net alias 出现在报告和 Tcl 中。

示例：

```text
input a, b, c;
output y;

eco_ab = a & b;
y = eco_ab | c;
```

报告/Tcl 中会优先使用 `eco_ab` 作为该中间 net 的名称。

### 表达式

支持的表达式形式：

| 语法 | 说明 |
|---|---|
| `0`, `1` | 单 bit 常量 |
| `4'b0111` | 二进制向量字面量 |
| `8'hff` | 十六进制向量字面量 |
| `6'd42` | 十进制向量字面量 |
| `a` | 标量信号引用 |
| `state[3]` | bit select |
| `state[3:0]` | range select，仅用于向量比较场景 |
| `!a`, `~a` | 取反 |
| `a & b` | AND |
| `a \| b` | OR |
| `a ^ b` | XOR |
| `a == b` | 等值比较 |
| `a != b` | 不等比较 |
| `sel ? a : b` | 1-bit mux |
| `(expr)` | 显式括号 |

运算符优先级从高到低：

1. 括号、常量、信号引用
2. `!`、`~`
3. `==`、`!=`
4. `&`
5. `^`
6. `|`
7. `?:`

### 向量语义

向量会在 elaboration 阶段展开为单 bit AIG 输入或输出。例如：

```text
input state[3:0];
output decoded;

decoded = (state == 4'b0111);
```

会展开成对 `state[0]` 到 `state[3]` 的逐 bit 比较，再把每一位 equality 结果 AND 起来。

当前支持：

- `state[i]` 作为标量表达式使用。
- `state == 4'b0111` 这类向量 equality/inequality。
- `state[3:0] == 4'b0111` 这类 range equality/inequality。

当前限制：

- 不支持 `y[3:0] = expr;` 这种多 bit 赋值。
- 不支持把整个向量直接作为标量表达式使用，例如 `y = state;`。
- `?:` 的 selector 必须是 1 bit，两个分支当前也必须是 1 bit。
- 不支持算术、移位、always/case、寄存器、模块层次或 Verilog 语法。

## 单元库格式

单元库使用 TOML 描述。每个 `[[cell]]` 定义一个 cell：

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
name = "MUX2"
inputs = ["I0", "I1", "S"]
output = "Z"
function = "S ? I1 : I0"
```

字段说明：

| 字段 | 说明 |
|---|---|
| `name` | cell 名称，必须唯一 |
| `inputs` | 输入 pin 名称列表，顺序就是报告和 Tcl 中使用的 pin 顺序 |
| `output` | 输出 pin 名称 |
| `function` | cell 逻辑函数，使用 DSL 表达式语法 |

库函数约束：

- `function` 只能引用 `inputs` 中列出的 pin。
- cell 输入数需要在 1 到 6 之间才能参与匹配。
- `function` 中不允许使用向量字面量或 bit select。
- 重复 cell 名称会报错。
- 空库会报错。

## 映射模型

`opt-cells` 的核心目标是减少总 cell 数。流水线如下：

```text
input.dsl
  -> parse DSL
  -> elaborate to AIG
  -> enumerate k-cuts
  -> match cuts against TOML cell library by NPN class
  -> choose a low-cell-count covering
  -> render report and Tcl
```

### AIG

内部使用 And-Inverter Graph：

- primary input 表示外部输入。
- `AND2` 是唯一的真实逻辑节点。
- edge 上带 invert bit。
- `OR`、`XOR`、`EQ`、`MUX` 等表达式都会 lowering 到 AIG。
- AIG 构建时会做 hash-consing，相同 AND 子图会复用节点。

### k-cut 和 NPN matching

映射时枚举每个 AIG 节点的 cut，并为 cut 计算 truth table。单元库中的每个 cell 也会从 `function` 计算 truth table，再建立 NPN canonical index。

NPN matching 允许：

- 输入 pin permutation。
- 匹配 cell 本身实现的输入反相或输出反相逻辑。

NPN matching 不表示“可以免费在任意 pin 前加反相器”。额外反相仍然需要真实 cell。

### 反相成本规则

这是项目里很重要的行为：

- 如果 cell 的 `function` 自带反相输入，例如 `!(!A1 | B1 | B2 | B3)`，这种反相是 cell 自身逻辑的一部分，不额外计 cell。
- 如果映射需要某个信号的反相信号，而库里没有 native inverted-input cell 或 native inverted-output cell 可以直接覆盖，就必须插入真实 `INV` cell。
- 如果需要反相但库中没有 `INV` 类 cell，并且没有其他 native cell 能覆盖该极性，映射会失败。

例子：

```text
input a, b;
output y;
y = !(a & b);
```

- 库中有 `NAND2` 时：映射为 1 个 `NAND2`。
- 库中只有 `AND2 + INV` 时：映射为 1 个 `AND2` 加 1 个 `INV`。
- 库中既没有 `NAND2` 也没有可用 `INV` 时：如果需要反相，映射报错。

### 两阶段 covering

当前 mapper 分两阶段：

1. `phase1` 自底向上估算每个节点正/负极性的最低 cell cost。
2. `phase2` 从 primary outputs 出发，自顶向下提交实际需要的 cell instance，并共享已经提交的 `(node, polarity)`。

这是一种实用的 DP covering 策略，不是全局 ILP 最优。它能稳定处理常见组合逻辑，并通过测试保证映射后 netlist 与原 AIG 逻辑等价。

此外，mapper 中有一个小的 library-aware rewrite：当库中存在适合的 3 输入正 cube cell 和 4 输入混合极性 cube cell 时，会尝试重排特定 6 项 AND cube，以暴露更少 cell 的 `AN3 + INR4` 这类覆盖。

## 输出报告

完整报告包含四部分：

1. Summary：输入文件、单元库路径、总 cell 数。
2. Mapped netlist：每个 mapped cell 的实例号、cell 类型、pin 连接和输出 net。
3. Tcl script：用于 ECO 流程的 `create_cell`、`create_net`、`connect_net` 命令。
4. Cell usage：按 cell 类型统计使用数量。

示例：

```text
===============================================================
  opt-cells mapping report
===============================================================
  Input file       : tests/fixtures/inputs/nand.dsl
  Cell library     : tests/fixtures/libs/basic.toml
  Total cells used : 1
---------------------------------------------------------------
  Mapped netlist
---------------------------------------------------------------
  u0 : NAND2  (a=a, b=b)  -> y

---------------------------------------------------------------
  Tcl script
---------------------------------------------------------------
create_cell     eco_NAND2_u0                           [get_lib_cells */NAND2]

connect_net     a                                      [get_pin eco_NAND2_u0/a]
connect_net     b                                      [get_pin eco_NAND2_u0/b]
connect_net     y                                      [get_pin eco_NAND2_u0/y]

---------------------------------------------------------------
  Cell usage
---------------------------------------------------------------
  NAND2 x 1
===============================================================
```

### Tcl 命名规则

- cell instance 名称固定为 `eco_<CELL>_u<uid>`，例如 `eco_NAND2_u0`。
- 默认内部 net 名称固定为 `eco_n<node>_u<uid>`，例如 `eco_n3_u1`。
- 非 primary output 的 mapped cell 输出会生成 `create_net`。
- primary output 对应的 net 不会额外生成 `create_net`。
- pin 名称来自 TOML library 的 `inputs` 和 `output` 字段。
- 中间信号只有以 `eco_` 开头时才会作为 Tcl/report alias 保留。
- 常量输入会以 `0` 或 `1` 作为 net 名称。

## Rust API

除了 CLI，项目也暴露 `run_pipeline`：

```rust
use opt_cells::{run_pipeline, RunInputs};

let (report, display_names) = run_pipeline(RunInputs {
    input_path: "inline.dsl".to_string(),
    input_text: "input a, b; output y; y = !(a & b);".to_string(),
    library_path: "tests/fixtures/libs/basic.toml".to_string(),
})?;
```

返回值：

- `report`：完整文本报告。
- `display_names`：内部信号名到用户可读名字的映射，主要用于向量 bit 显示。

## 测试与验证

常用检查：

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```

测试覆盖包括：

- parser 和 AST 基础语法测试。
- elaboration 语义测试，例如向量展开、宽度不匹配、未声明信号、共享子表达式。
- library loader 测试，例如重复 cell、未知 pin、truth table 计算。
- mapper 行为测试，例如 native NAND 优先、INV 计数、缺 INV 报错、内建反相 pin 不额外计数。
- golden report 测试，确保输出格式稳定。
- property tests，验证 NPN canonical 的性质以及随机小 AIG 映射后的功能等价。
- workspace-level 测试，检查 Tcl 生成命令和 netlist 一致性。

CI 位于 `.github/workflows/ci.yml`，会执行 `fmt`、`clippy` 和 `cargo test`。

更新 golden 文件时可以使用测试中的 `UPDATE_GOLDEN=1` 约定：

```powershell
$env:UPDATE_GOLDEN = "1"
cargo test --test golden
```

## 当前限制

- 只处理组合逻辑，不支持寄存器、时钟、复位或时序约束。
- 不解析 Verilog、SystemVerilog、Liberty 或 LEF/DEF。
- 优化目标只有 cell 数，不考虑面积、delay、power 或 fanout。
- cell matching 目前限制在最多 6 输入。
- multi-bit mux 和 multi-bit assignment 未实现。
- 输出格式是文本报告和 Tcl 脚本，没有 JSON、Verilog netlist 或图形化输出。
- covering 使用实用 DP 策略，不保证 DAG sharing 场景下的严格全局最优。

## License

`Cargo.toml` 声明为 `MIT OR Apache-2.0`。
