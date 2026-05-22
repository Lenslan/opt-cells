# opt-cells — Design Spec

**Date**: 2026-05-21
**Status**: Approved for implementation planning
**Audience**: Implementer of the Rust tool

---

## 1. Purpose

`opt-cells` is a Rust CLI tool that takes a combinational logic expression written in a custom DSL plus a cell library description, and emits the **minimum-cell-count** mapping of the logic onto cells from the library.

It is positioned as an **industrial-grade synthesizer subset** — meaningfully more capable than a teaching toy, but explicitly narrower than ABC/Yosys/Synopsys DC.

### 1.1 Motivating examples

- Input `y = !(a & b);` with a library containing `NAND2` should yield a 1-cell mapping (one `NAND2`), not 2 cells (`AND2` + `INV`).
- Input `decoded = (state[3:0] == 4'b0111);` with a library containing `AND4` should yield a 1-cell mapping (one `AND4` with the highest-bit input inverted), not 4+ cells.

The user does **not** need to enumerate input-inversion variants of cells in the library — NPN-equivalence matching handles that automatically.

---

## 2. Scope & non-goals

### In scope
- DSL for combinational logic (boolean ops, bit indexing, bit-vector literals, equality comparison)
- Custom TOML cell library format
- AIG-based intermediate representation
- k-cut enumeration (k=6) + NPN canonical matching + two-phase DP covering
- Human-readable text report output
- Strong diagnostics via `ariadne`
- Comprehensive testing (unit + integration + proptest + golden)

### Explicitly out of scope (YAGNI for v1)
- Sequential logic, registers, flip-flops
- Verilog parsing — uses custom DSL instead
- Liberty (.lib) parsing — uses custom TOML format
- Area or delay optimization — only cell count
- Multi-bit assignment `y[3:0] = expr` — write 4 single-bit assignments
- Multi-output JSON/Verilog netlist output — text report only
- Module hierarchy
- ILP-based optimal covering (using DP heuristic instead)
- Warning/severity system — every problem is either an error or a one-line informational note (see §8.5)

---

## 3. Architecture

### 3.1 Module layout (single crate)

```
┌──────────────────────────────────────────────────────────┐
│  cli       — argv parsing, file IO, error rendering      │
├──────────────────────────────────────────────────────────┤
│  report    — format mapping result into text report      │
├──────────────────────────────────────────────────────────┤
│  mapper    — two-phase DP covering                       │
├──────────────────────────────────────────────────────────┤
│  match_npn — NPN canonical form + library index          │
├──────────────────────────────────────────────────────────┤
│  aig       — AIG data structure + k-cut enumeration      │
├──────────────────────────────────────────────────────────┤
│  frontend  — DSL parser → AST → AIG; library loader      │
└──────────────────────────────────────────────────────────┘
```

Single Rust crate (`opt-cells`). All non-CLI logic lives in `lib.rs` modules; binary `main.rs` is just CLI wiring. May split into `opt-cells` + `opt-cells-core` later if a second consumer (GUI/REPL/web) appears.

### 3.2 Data flow

```
input.dsl  ──parse──► AST ──elaborate──► AIG
                                          │
library.toml ──load──► CellLib ──npn-index──► NpnLibIndex
                                          │              │
                                          ▼              ▼
                                       k-cut enum      lookup
                                          │              │
                                          └──────► Mapper (DP)
                                                       │
                                                       ▼
                                                  MappedNetlist
                                                       │
                                                       ▼
                                                    Report
```

### 3.3 Boundaries

- `frontend` knows only about DSL and AIG construction; does not see library or matcher
- `aig`/`match_npn`/`mapper` are file-format agnostic; consume a `CellLib` value
- `report` consumes `MappedNetlist` only; never touches AIG
- Pipeline is single-process, pure-functional, no shared state — easy to test, easy to swap layers later

### 3.4 Dependencies

| Crate | Purpose |
|---|---|
| `clap` (derive) | CLI parsing |
| `serde` + `toml` | Cell library deserialization |
| `chumsky` | DSL parser (good error reporting) |
| `ariadne` | Diagnostic rendering with source spans |
| `thiserror` | Error type derivation |
| `anyhow` | CLI-boundary error type |
| `proptest` (dev) | Property-based tests |

---

## 4. DSL specification

### 4.1 Grammar (EBNF)

```ebnf
program     := input_decl* output_decl* statement+

input_decl  := "input"  ident ("[" int ":" int "]")? ";"
output_decl := "output" ident ("[" int ":" int "]")? ";"

statement   := lvalue "=" expr ";"
lvalue      := ident ("[" int "]")?           ; single-bit index only (no range)

expr        := or_expr
or_expr     := xor_expr ("|" xor_expr)*
xor_expr    := and_expr ("^" and_expr)*
and_expr    := eq_expr  ("&" eq_expr)*
eq_expr     := unary    (("==" | "!=") unary)?
unary       := ("!" | "~") unary | primary
primary     := "(" expr ")" | literal | signal_ref
signal_ref  := ident ("[" int (":" int)? "]")?
literal     := int "'" base digits          ; e.g. 4'b0111, 8'hFF
            | "0" | "1"                     ; single-bit literal
```

### 4.2 Semantics

- `a == b` for single-bit operands → `!(a ^ b)`
- `a == b` for vector operands → AND of bitwise equalities
- Vectors are desugared into single-bit signals during elaboration: `state[3:0]` becomes 4 distinct primary inputs internally
- Multiple `output = expr;` statements may share subexpressions; AIG hash-consing automatically deduplicates

### 4.3 Restrictions (rejected with clear error)

- Multi-bit assignment `y[3:0] = expr` — user must write 4 single-bit assignments
- Width mismatch in expressions — e.g. `y = state` where widths differ
- Undeclared signal references
- Comparisons on the left-hand side

### 4.4 What is intentionally NOT in the DSL

- `always`, `if`/`case`, `wire` declarations
- Arithmetic (`+`, `-`, `*`)
- Shifts (`<<`, `>>`)
- Signed/unsigned distinction
- Module hierarchy

### 4.5 Examples

```
input  a, b;
input  state[3:0];
output y;
output decoded;

y       = !(a & b);
decoded = (state == 4'b0111);
```

---

## 5. Cell library format

### 5.1 Schema (TOML)

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

### 5.2 Key design decisions

1. **`function` reuses the DSL expression syntax** — single parser handles both
2. **No variant enumeration needed** — NPN matching covers input inversions/permutations automatically; only base form is written in the library
3. **No area/delay fields** — cell-count optimization doesn't need them; reserved for future
4. **k=6 input limit** — cells with >6 inputs cannot be matched (cut enumeration is bounded at k=6); loader emits a one-line informational note but loads the rest of the library

### 5.3 Loading pipeline

```
library.toml
   │
   ├─ TOML decode → Vec<CellDecl>
   ├─ Parse each cell.function via DSL parser → small AIG
   ├─ Compute truth table for each cell (u64 since k≤6)
   ├─ Compute NPN canonical form of each truth table
   └─ Build index: HashMap<NpnClass, Vec<InputMapping>>
```

### 5.4 `InputMapping` sketch

```rust
struct InputMapping {
    cell_id: CellId,
    n_inputs: u8,                  // valid prefix length of pin_perm
    pin_perm: [u8; 6],              // cut-input i -> cell-pin pin_perm[i]
    input_negation: u8,            // bitmask: cut-input i negated before cell pin
    output_negation: bool,         // cell output negated to reach target truth table
}
```

### 5.5 Validation

- Cell names unique (duplicate is an error with both source locations)
- `function` may only reference variables listed in `inputs`
- 0 inputs or >6 inputs: prints a one-line informational note (cell is loaded but unmatchable); not an error
- Empty library: rejected
- Missing INV-class cell: load succeeds, but flagged at the per-mapping level if inversion is actually needed

---

## 6. Algorithm

### 6.1 AIG construction

AIG has two node kinds: **Primary Input** and **AND2**. Every edge carries an inversion bit. Constants 0/1 are represented as a special const node + polarity.

```rust
struct AigNode { kind: NodeKind }                  // PI | And2 { l: Edge, r: Edge }
struct Edge    { node: NodeId, invert: bool }
```

**Hash-consing**: when constructing an AND2, normalize children order (smaller `NodeId` on the left), then look up in a hashmap keyed by `(l.node, l.invert, r.node, r.invert)`. Hit → reuse existing node.

**Operator lowering**:
- `a | b` → `!(!a & !b)`
- `a ^ b` → `!(!(a & !b) & !(!a & b))`
- `a == b` (single bit) → `!(a ^ b)`
- vector `==` → AND of bitwise `==`

### 6.2 k-cut enumeration (k=6)

For each node n, compute `cuts(n)` = subgraphs rooted at n with ≤6 leaves:

```
PI:        cuts(PI) = { {PI} }
AND2(l,r): cuts(n)  = { {n} } ∪ { cl ∪ cr | cl ∈ cuts(l), cr ∈ cuts(r), |cl ∪ cr| ≤ 6 }
```

**Pruning**:
- Keep at most N=8 cuts per node (sorted by leaf count + truth-table norm)
- Drop dominated cuts (cut A's leaves ⊆ cut B's leaves → B dropped)
- Always keep trivial cut (guarantees feasibility)

Complexity: O(N²) per node, linear traversal — millisecond-scale on AIGs up to ~1000 nodes.

### 6.3 NPN canonical form

For each cut, compute its truth table (k≤6 fits in `u64`), then reduce to NPN canonical:

- Enumerate all (input negation × input permutation × output negation) combinations — at most 2^6 × 6! × 2 = 92160 for k=6
- Apply each combination to the truth table
- Canonical = lexicographically smallest resulting truth table

Library index: `HashMap<u64 npn_canonical, Vec<InputMapping>>`. Cut matching = compute cut's NPN canonical → hashmap lookup → list of viable (cell, permutation, negation) tuples.

### 6.4 Two-phase DP covering

Strict optimum (min cell count with DAG sharing) is NP-hard. Use the standard two-phase heuristic:

#### Cost model assumption (important)

**Input pin inversion is free.** When NPN matching says a cell can implement a cut with one or more of its cut-leaves arriving inverted, we treat that as a single-cell mapping — the report shows the inversion as `pin=!signal` annotation, and no separate INV cell is counted.

Rationale: real standard-cell libraries typically supply cells with built-in inverted input pins. Concrete example — TSMC's `INR4D0BWP7T40P140` has native function `out = !(!A1 | B1 | B2 | B3)`, where the inversion on pin `A1` is part of the cell's physical definition. So a logic like `!s3 & s2 & s1 & s0` maps to exactly one such cell, without any external inverter. The user is responsible for populating their library with the variants they want available (the tool does not invent cells); NPN matching only finds equivalence between user-provided cells and the target cut.

INV cells are counted only when explicit polarity reconciliation is needed at a primary output (or at a fanout point where two parent cuts need opposite polarities and no cheaper option exists).

#### Phase 1 — bottom-up estimation

In topological order, for each AIG node `n`, compute and record:

- `best_cost_pos[n]`, `best_cut_pos[n]`, `best_mapping_pos[n]` — best (cost, cut, mapping) producing positive polarity
- `best_cost_neg[n]`, `best_cut_neg[n]`, `best_mapping_neg[n]` — best (cost, cut, mapping) producing negative polarity

For each cut `c` at `n` and each mapping `m` matching `c`:

```
mapping_cost(c, m) = 1 + Σ leaf in c.leaves: leaf_cost(leaf, m.input_negation[leaf])

leaf_cost(n, want_negated) =
    if want_negated { best_cost_neg[n] } else { best_cost_pos[n] }

best_cost_pos[n] = min over (c, m) with m.output_negation == false : mapping_cost(c, m)
best_cost_neg[n] = min(
                     min over (c, m) with m.output_negation == true  : mapping_cost(c, m),
                     best_cost_pos[n] + 1                            (insert INV after positive form)
                   )
```

PI cost = 0 in both polarities (external).

This phase double-counts shared leaves — corrected in phase 2.

#### Phase 2 — top-down commit

```
Initialize: required = {} ; queue = primary outputs annotated with required polarity

While queue not empty:
  take (node n, polarity p) from queue
  if (n, p) already in required: continue
  add (n, p) to required
  if n is PI: continue                                    // PI is just a wire
  look up best_cut_*[n] and best_mapping_*[n] for polarity p
  if the recorded best for polarity p was "INV after positive":
    add (n, positive) to queue                            // need the positive form
    record an INV cell at n for the polarity flip
  else:
    record cell instance = (chosen mapping)
    for each leaf in chosen cut:
      add (leaf, leaf's required polarity per mapping.input_negation) to queue
```

Final cell count = #(cell instances recorded) + #(INV cells recorded for polarity flips).

#### Known suboptimality

Phase 1 estimates a cut's cost assuming every leaf is materialized solely for this cut; in reality leaves may be shared across multiple parent cuts, so the true cost is lower than Phase 1 estimates. Phase 2 still picks the cut that Phase 1 ranked best — it does not re-estimate with sharing in mind. Acceptable for v1; could add a refinement iteration later.

---

## 7. Output report format

Three sections: summary, mapped netlist, cell usage.

### 7.1 Example — `y = !(a & b)`

```
═══════════════════════════════════════════════════════════════
  opt-cells mapping report
═══════════════════════════════════════════════════════════════
  Input file       : examples/nand.dsl
  Cell library     : libs/basic.toml
  Total cells used : 1
───────────────────────────────────────────────────────────────
  Mapped netlist
───────────────────────────────────────────────────────────────
  u0 : NAND2  (a=a, b=b)  -> y

───────────────────────────────────────────────────────────────
  Cell usage
───────────────────────────────────────────────────────────────
  NAND2 × 1
═══════════════════════════════════════════════════════════════
```

### 7.2 Example — `decoded = (state[3:0] == 4'b0111)`

```
═══════════════════════════════════════════════════════════════
  opt-cells mapping report
═══════════════════════════════════════════════════════════════
  Input file       : examples/decode.dsl
  Cell library     : libs/basic.toml
  Total cells used : 1
───────────────────────────────────────────────────────────────
  Mapped netlist
───────────────────────────────────────────────────────────────
  u0 : AND4  (a=!state[3], b=state[2], c=state[1], d=state[0])  -> decoded

───────────────────────────────────────────────────────────────
  Cell usage
───────────────────────────────────────────────────────────────
  AND4 × 1
═══════════════════════════════════════════════════════════════
```

### 7.3 Formatting rules

- Input inversion shown inline as `pin=!signal` (cell absorbs the inversion — see §6.4 cost-model assumption; no extra INV cell is counted)
- Vector signals restored from internal `state__3` back to `state[3]` for display (frontend maintains the original-name mapping)
- Intermediate nodes named `u0, u1, u2, ...` in topological order
- Multi-cell example:
  ```
  u0 : NAND2  (a=x, b=y)         -> n1
  u1 : INV    (a=n1)             -> z1
  u2 : AND2   (a=n1, b=z2)       -> out
  ```
- Verbose mode is not implemented; the `-v` flag was removed during cleanup. (A future per-node Debug section listing candidate cuts, NPN classes, matched cells, and chosen cell may be reintroduced if needed.)

### 7.4 Failure modes in the report

- Library missing INV but inversion needed → error with the specific signal name
- A node has no matching cell at all → error listing the function's truth table and asking which cell category is missing

---

## 8. Error handling

### 8.1 Error type hierarchy

```rust
pub enum OptCellsError {
    Io(#[from] std::io::Error),
    ParseDsl(ParseError),       // span + expected tokens
    ParseLibrary(LibError),     // TOML or function-parse failure
    Elaborate(ElabError),       // undeclared signal, width mismatch, multi-bit lhs
    Mapping(MapError),          // library lacks critical cell, no cut matches
}
```

### 8.2 Rendered diagnostic example

```
error: undefined signal `state`
  ┌─ examples/decode.dsl:3:11
  │
3 │   decoded = (state == 4'b0111);
  │             ^^^^^ not declared as input or earlier output
  │
help: add `input state[3:0];` at the top of the file
```

Rendered via `ariadne` from a span carried through parse, elaborate, and mapping phases.

### 8.3 Required diagnostic scenarios

| Scenario | Diagnostic |
|---|---|
| DSL uses undeclared signal | `undefined signal 'X'` + suggest input decl |
| Vector width mismatch | `width mismatch: lhs is 4 bits, rhs is 3 bits` |
| Multi-bit assignment `y[3:0] = expr` | `multi-bit assignment not supported; write 4 single-bit assignments` |
| Duplicate cell name | `duplicate cell name 'NAND2' at library.toml:lines 12, 27` |
| `function` references unknown var | `cell 'AOI21' function references 'd' which is not in inputs list` |
| Inversion needed but no INV cell | `cannot synthesize: signal '!n3' requires inversion but library has no inverter cell` |
| Node has no matching cell | `node u7 (function 0x6996) has no matching cell; library does not cover XOR-like functions` |

### 8.4 Error boundaries

- `frontend`, `aig`, `match_npn`, `mapper` return `Result<_, SpecificError>`
- `cli` converts to `OptCellsError`, renders via `ariadne`, exits 1
- `panic!` reserved for internal invariant violations (real bugs, not user input)

### 8.5 Not doing

- No warnings — every problem is an error (except cell library cells with input count outside [1,6], which print an informational note about being unmatchable)
- No autofix — diagnostics suggest, never modify user files

---

## 9. Testing strategy

### 9.1 Unit tests (per-module)

| Module | Focus |
|---|---|
| `frontend::parser` | every grammar production succeeds/fails, error spans accurate |
| `frontend::elaborate` | vector flattening, shared subexpression detection, multi-bit lhs rejection |
| `aig` | hash-consing (same expression built twice yields same `NodeId`), De Morgan lowering correctness |
| `aig::cuts` | exact cut lists for hand-checked small graphs |
| `match_npn` | NPN canonical invariant (all 64 NPN-class variants map to same canonical) |
| `mapper` | on hand-built tiny libraries, the chosen cut set is correct |

### 9.2 Integration tests (golden)

```
tests/
  fixtures/
    libs/
      basic.toml          # INV, AND2, OR2, NAND2, NOR2
      with_aoi.toml       # + AOI21, OAI21
      with_and4.toml      # + AND4 (validates the user's second example)
    inputs/
      nand.dsl            # y = !(a & b)
      decode.dsl          # decoded = (state == 4'b0111)
      mux.dsl
      shared.dsl          # multiple outputs share subexpressions
    expected/
      nand_basic.txt
      decode_and4.txt
      ...
```

Each `.dsl × library` combination runs end-to-end and is compared to the golden text file in `expected/`.

**Required golden tests** (must pass):
- `nand.dsl + basic.toml` → 1 NAND2
- `nand.dsl + (only AND2 + INV)` → 1 AND2 + 1 INV
- `decode.dsl + with_and4.toml` → 1 AND4 (core validation of motivating example #2)
- `decode.dsl + basic.toml` → multiple AND2 + 1 INV; cell count computed by hand and frozen

### 9.3 Property tests (`proptest`)

Random small AIGs (≤20 nodes) and random small libraries verify:
- **Functional equivalence**: simulating mapped netlist on all input combinations equals simulating original AIG
- **NPN involution**: `canonical(canonical(T)) == canonical(T)` for random truth tables
- **Monotonicity**: adding a cell to the library never increases the optimal cost

### 9.4 Test simulator (test-only infrastructure)

A minimal AIG/netlist simulator under `tests/common/sim.rs` (not production code) — given a cell library's function strings and an input assignment, computes outputs in topological order. Drives the functional-equivalence proptests.

### 9.5 CI

Single GitHub Actions workflow:
- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`

MSRV pinned (e.g. 1.75).

### 9.6 Not doing

- No fuzz testing (valuable but not v1)
- No benchmarks (performance not a current concern; add `criterion` later if needed)

---

## 10. CLI & UX

### 10.1 Surface

```
opt-cells [OPTIONS] --library <FILE> <INPUT>

Arguments:
  <INPUT>  Path to .dsl input file, or "-" for stdin

Options:
  -l, --library <FILE>   Cell library TOML file        [required]
  -o, --output <FILE>    Write report to file (default: stdout)
  -q, --quiet            Only show cell-count summary
  -h, --help
  -V, --version
```

### 10.2 Typical invocations

```bash
opt-cells -l libs/basic.toml examples/decode.dsl
echo 'input a, b; output y; y = !(a & b);' | opt-cells -l libs/basic.toml -
```

### 10.3 Exit codes

- `0` success
- `1` user error (DSL/library syntax, semantic, mapping infeasibility)
- `2` CLI arg error (clap default)
- `101` panic (internal bug; rustc default)

### 10.4 Modes

- **Default**: full report (summary + netlist + cell usage)
- **`-q` quiet**: single line `total cells: N` — for batch evaluation scripts

### 10.5 Stdin

When `<INPUT>` is `-`, read DSL from stdin. Enables pipeline use.

### 10.6 No interactive mode, no REPL

One-shot tool. Future GUI/REPL would be a separate binary on the same core modules.

---

## 11. Build & release

- Single binary `opt-cells`
- Single crate `opt-cells` (split into `opt-cells-core` later if a second consumer appears)
- `cargo install` from local checkout for initial use
- No release artifacts or distribution channel for v1

---

## 12. Open questions for implementation

None blocking. Implementation may surface the following minor decisions:

- Concrete pruning threshold N for cuts per node (proposed 8; may tune)
- Exact NPN canonical algorithm (proposed brute force; may switch to Roman Lyakh's algorithm if profiling shows hotspot)
- Whether to emit `u0/u1/...` node names or `g0/g1/...` — purely cosmetic
