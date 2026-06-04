# opt-cells

`opt-cells` 是一个 Rust 命令行工具，用于把组合逻辑 DSL 映射到 TOML 单元库中的最少单元数实现。

工具会输出一份文本报告，包含 mapped netlist、可用于 ECO 流程的 Tcl 脚本，以及 cell usage 统计。

## 使用方式

```powershell
cargo run -- tests/fixtures/inputs/nand.dsl -l tests/fixtures/libs/basic.toml
```

将报告写入文件：

```powershell
cargo run -- tests/fixtures/inputs/nand.dsl -l tests/fixtures/libs/basic.toml -o report.txt
```

只输出 cell 数量摘要：

```powershell
cargo run -- tests/fixtures/inputs/nand.dsl -l tests/fixtures/libs/basic.toml --quiet
```

## 输入 DSL

```text
input a, b;
output y;
y = !(a & b);
```

支持的表达式操作符包括 `!`、`&`、`|`、`^`、`==`、`!=`。也支持 vector input 和 bit select，例如：

```text
input state[5:0];
output decoded;
decoded = (state == 6'b101000);
```

## 单元库

单元库使用 TOML 描述，每个 `[[cell]]` 定义一个 cell：

```toml
[[cell]]
name = "NAND2"
inputs = ["a", "b"]
output = "y"
function = "!(a & b)"
```

`inputs` 的顺序就是 mapper 和 Tcl 脚本使用的 library pin 顺序。

## 报告内容

报告包含三个主要部分：

- `Mapped netlist`：mapped cell、pin 连接和输出 net。
- `Tcl script`：根据 mapped netlist 生成的 ECO Tcl 命令。
- `Cell usage`：每类 cell 的使用数量。

生成的 Tcl 遵循以下规则：

- 所有 mapped cell 都通过 `create_cell` 创建。
- 所有非 primary output 的中间 net 都通过 `create_net` 创建。
- 创建的 instance 名称固定以 `eco_` 开头，例如 `eco_NAND2_u0`。
- 创建的中间 net 名称固定以 `eco_` 开头，例如 `eco_n3_u1`。
- `connect_net` 的 pin 名称来自 TOML cell library 中定义的 pin 名称。
- 命令列、net/instance 列会做固定宽度对齐，方便阅读和 diff。

`tests/fixtures/inputs/STATE_1.dsl` 对应的参考 Tcl 风格见 `tests/fixtures/tcl/state_1.tcl`。
