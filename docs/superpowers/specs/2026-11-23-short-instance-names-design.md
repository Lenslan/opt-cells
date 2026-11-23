# 实例名只保留功能码 — Design Spec

**Date**: 2026-11-23
**Status**: Implemented

## 1. 目标

Tcl 脚本中的 cell 实例名是 `eco_<CELL>_u<uid>`，`<CELL>` 为库里的完整 cell 名。标准单元的
名字在功能码之后还带有驱动强度和工艺信息（`AO21D0BWP7T40P140` = `AO21` + `D0` +
`BWP7T40P140`）。这一段对辨认实例没有帮助，却出现在每个实例名和每条 `get_pin` 路径里：

```text
create_cell     eco_AO21D0BWP7T40P140_u0               [get_lib_cells */AO21D0BWP7T40P140]
connect_net     a                                      [get_pin eco_AO21D0BWP7T40P140_u0/A1]
```

本次让实例名只保留功能码：

```text
create_cell     eco_AO21_u0                            [get_lib_cells */AO21D0BWP7T40P140]
connect_net     a                                      [get_pin eco_AO21_u0/A1]
```

不在范围内：修改库文件格式；缩短报告其它位置的 cell 名；保留旧实例名的开关；net 的命名。

## 2. 行为

### 功能码

功能码是 cell 名中第一个 `D<数字>BWP` 片段之前的部分。`<数字>` 是一位或多位十进制数字，
匹配区分大小写。名字里没有这个片段，或者片段位于名字开头（功能码会是空串）时，功能码就是
完整的 cell 名。

| cell 名 | 功能码 | |
|---|---|---|
| `AO21D0BWP7T40P140` | `AO21` | |
| `INVD0BWP7T40P140` | `INV` | |
| `ND2D0BWP7T40P140` | `ND2` | `D2` 后面不是 `BWP`，不是截断点 |
| `MUX2ND0BWP7T40P140` | `MUX2N` | |
| `AO21D16BWP7T40P140LVT` | `AO21` | 多位驱动强度、VT 后缀 |
| `NAND2`、`AND2B1` | 原名 | 不含 `D<数字>BWP` |
| `D0BWP7T40P140` | 原名 | 片段之前没有内容 |

`workspace/lib/basic.toml` 的 39 个 cell 全部符合 `<功能码>D0BWP7T40P140`，得到 39 个互不
相同的功能码。

规则认不出的命名方式不会出错，只是不缩短：实例名仍是完整 cell 名，与改动前相同。

### 输出

| 位置 | 改动前 | 改动后 |
|---|---|---|
| `create_cell` 的实例名 | `eco_AO21D0BWP7T40P140_u0` | `eco_AO21_u0` |
| `[get_pin <实例>/<pin>]` | `eco_AO21D0BWP7T40P140_u0/A1` | `eco_AO21_u0/A1` |
| `[get_lib_cells */<CELL>]` | `*/AO21D0BWP7T40P140` | 不变 |
| Mapped netlist 段 | `u0 : AO21D0BWP7T40P140  (…)` | 不变 |
| Cell usage 段 | `AO21D0BWP7T40P140 x 1` | 不变 |
| 内部 net、DSL 别名、端口 | `eco_n3_u1`、`eco_ab`、`y` | 不变 |

- 默认生效，没有开关。
- `get_lib_cells` 引用的必须是库里真实存在的名字，所以保持完整。Mapped netlist 和 Cell usage
  用来核对实际使用的 cell 型号，也保持完整。
- `--prefix test` 照常叠加：`eco_test_AO21_u0`。
- Tcl 各列的对齐宽度不变，实例名变短后由空格补齐。
- 库里的 cell 名都不含 `D<数字>BWP` 时（例如只有 `NAND2`、`AND2` 这类名字的库），输出与改动前
  逐字节一致。

### 唯一性

uid 在一次运行内唯一，实例名以 `_u<uid>` 结尾，所以缩短不会让同一次运行中的两个实例重名。
库里同时有同一功能码、不同驱动强度的两个 cell（`AN2D0…` 与 `AN2D4…`）时也是如此：实例名是
`eco_AN2_u0`、`eco_AN2_u1`，各自的 `get_lib_cells` 指向各自的完整名字。

不同运行之间的实例名本来就会相同（同一个 cell、同一个 uid），仍然用 `--prefix` 区分。

## 3. 方案

源码只改 `src/report/mod.rs`：

- 新增私有函数 `function_code(cell_name: &str) -> &str`，实现上面的规则。
- `instance_name` 用 `function_code(cell_name)` 代替 `cell_name`。`create_cell` 和 `connect_net`
  两处的实例名都由 `instance_name` 生成，所以始终一致。

库格式、`CellDecl`、CLI 参数、`RunInputs` / `ReportInput` 都不变。

否决的方案：

- 在库文件里列出要去掉的后缀（`instance_name_strip = ["D0BWP7T40P140"]`）：库里每多一种驱动
  强度或 VT 就要多列一项，漏列的 cell 保持长名字。
- 每个 `[[cell]]` 增加 `short` 字段：最灵活，但每个 cell 都要多维护一个名字。
- 加载库时把功能码存进 `CellDecl`：功能码只有 report 使用，而十余处手工构造 `CellDecl` 的
  测试都要跟着改。

## 4. 验证

- `src/report/mod.rs` 单元测试：
  - `function_code` 对上表各类名字的结果；含两个片段的名字在第一个片段处截断；残缺片段
    （`AO21D0`、`AO21DBWP7T40P140`、`AO21D0BW`）、小写名字和空串保持原名。
  - `workspace/lib/basic.toml` 中以 `D0BWP7T40P140` 结尾的每个 cell（目前是全部 39 个）：功能码
    等于完整名去掉这个后缀。
  - 手工构造的 netlist：同一功能码、不同驱动强度的两个 cell 得到不同的实例名，`get_lib_cells`
    指向各自的完整名字。
- `tests/instance_names.rs`（经 `run_pipeline`）：
  - `y = (a & b) | c` 配 workspace 库的完整 Tcl 命令序列。
  - 同一个 cell 的两个实例由 uid 区分。
  - Mapped netlist 和 Cell usage 段仍显示完整 cell 名。
  - `--prefix` 与功能码叠加。
  - 同一个库里带片段和不带片段的 cell 同时出现时各自的实例名。
  - 较大的用例中，Mapped netlist 段的每个 `u<uid> : <CELL>` 都对应一条
    `create_cell eco_<功能码>_u<uid> [get_lib_cells */<CELL>]`，实例名互不相同，`get_pin`
    引用的实例与创建的实例一致。
- `tests/golden.rs` 新增 `state_1_state_test`（`STATE_1.dsl` × `state_test.toml`）：逐字节固定
  一份带片段 cell 名的完整报告，包括列对齐。原有 golden 文件不含带片段的 cell 名，没有变化。
- 现有测试中写死的长实例名（`tests/workspace_basic.rs`、`tests/strict_inversion.rs`、
  `tests/name_prefix.rs`）改为新名字。
- 变异检查，共 11 种改法，每一种都有测试失败：
  - 规则本身：不要求数字、不要求 `BWP`、只认一位数字、不区分大小写、取最后一个片段、片段在
    开头时返回空串、找到片段后不截断。
  - 规则的使用位置：Mapped netlist 段、Cell usage 段或 `get_lib_cells` 也改用功能码；
    `connect_net` 的实例名仍用完整 cell 名。
- 回归：14 个 DSL（全部 fixture 输入和 `workspace/` 下的 DSL）× 7 个库 × {默认、`--prefix`、
  `--quiet`}，共 294 次运行（228 次成功、66 次报错），对比改动前后的可执行文件：
  - 退出码和 stderr 全部相同。
  - 242 次的 stdout 逐字节相同，其中包括 cell 名都不含片段的 4 个库的全部运行。
  - 52 次的 stdout 有变化，共 720 行，全部是 `create_cell` 和 `connect_net` 行。这些 stdout 与
    “把改动前 stdout 中的实例名换成短名并重新补齐列宽”的结果逐字节相同。
