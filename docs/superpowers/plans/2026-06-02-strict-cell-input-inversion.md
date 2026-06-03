# Strict Cell Usage / No Free Input Inversion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop the mapper from treating input-pin inversion as free; every added inversion must be a real, counted `INV` cell (or a cell that natively produces the complemented function), while cell built-in inversions stay free via permutation matching.

**Architecture:** Keep the NPN matcher. Reinterpret each matched `input_negation` bit as "this leaf must be fed in its complemented polarity, which costs a real `INV` cell." Charge it in `phase1`, materialize it as a real cell in `phase2`, render produced signals (never `pin=!signal`) in `report`, and identify every signal by `(NodeId, polarity)`.

**Tech Stack:** Rust (single crate `opt-cells`), `cargo test`, golden text fixtures with `UPDATE_GOLDEN=1` regeneration, `proptest`.

**Spec:** `docs/superpowers/specs/2026-06-02-strict-cell-input-inversion-design.md`

---

## File Structure

Files modified (no new source modules; one new test file + fixtures):

- `src/mapper/netlist.rs` — rename `PinInput.invert`→`leaf_negated`, `CellInstance.produces_negation`→`output_negated`; a signal is `(aig_node, output_negated)`.
- `src/mapper/phase1.rs` — `run` gains `has_inv: bool`; complemented primary inputs cost a real `INV`; the "positive + INV" route is gated on `INV` availability.
- `src/mapper/phase2.rs` — `find_inv_cell` becomes `pub`; complemented primary inputs fall through to the `INV`-producing path instead of being free wires; pins reference produced `(leaf, polarity)` signals.
- `src/mapper/mod.rs` — compute `has_inv` and pass it to `phase1::run`.
- `src/report/mod.rs` — resolve each pin and primary output via `(NodeId, polarity) → producing cell`; never print `!` on an input pin.
- `tests/common/mod.rs` — simulate by `(NodeId, polarity)` signal so real `INV` cells are evaluated.
- `tests/strict_inversion.rs` — NEW behavioral tests (the TDD driver).
- `tests/fixtures/libs/no_inv.toml`, `tests/fixtures/libs/with_andb.toml`, `tests/fixtures/inputs/andb.dsl` — NEW fixtures.
- `tests/fixtures/expected/*.txt` — regenerated goldens.
- `docs/superpowers/specs/2026-05-21-opt-cells-design.md` — §6.4 / §7.3 corrected.

---

## Task 1: Core fix — charge real INV cells for added inversions

**Files:**
- Create: `tests/strict_inversion.rs`
- Create: `tests/fixtures/libs/no_inv.toml`
- Create: `tests/fixtures/libs/with_andb.toml`
- Create: `tests/fixtures/inputs/andb.dsl`
- Modify: `src/mapper/netlist.rs`
- Modify: `src/mapper/phase1.rs`
- Modify: `src/mapper/phase2.rs`
- Modify: `src/mapper/mod.rs`
- Modify: `src/report/mod.rs`
- Modify: `tests/common/mod.rs`
- Regenerate: `tests/fixtures/expected/{nand_basic,decode_and4,mux_basic,nand_and2_only}.txt`

---

- [ ] **Step 1: Create the test fixtures**

Create `tests/fixtures/libs/no_inv.toml` (AND2 + OR2, deliberately no inverter and no NAND/NOR):

```toml
[[cell]]
name = "AND2"
inputs = ["a", "b"]
output = "y"
function = "a & b"

[[cell]]
name = "OR2"
inputs = ["a", "b"]
output = "y"
function = "a | b"
```

Create `tests/fixtures/libs/with_andb.toml` (a cell with a built-in inverted input, plus an unused INV to prove it stays unused):

```toml
[[cell]]
name = "INV"
inputs = ["a"]
output = "y"
function = "!a"

[[cell]]
name = "AND2B1"
inputs = ["a", "b"]
output = "y"
function = "!a & b"
```

Create `tests/fixtures/inputs/andb.dsl`:

```
input x, y;
output o;
o = !x & y;
```

- [ ] **Step 2: Write the behavioral tests (TDD driver)**

Create `tests/strict_inversion.rs`:

```rust
mod common;

use opt_cells::{run_pipeline, RunInputs};

fn run(input_rel: &str, lib_rel: &str) -> String {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let input_path = format!("tests/fixtures/{}", input_rel);
    let library_path = format!("tests/fixtures/{}", lib_rel);
    let input_text =
        std::fs::read_to_string(format!("{}/tests/fixtures/{}", manifest, input_rel)).unwrap();
    run_pipeline(RunInputs {
        input_path,
        input_text,
        library_path,
    })
    .expect("pipeline ok")
    .0
}

#[test]
fn nand_uses_native_nand2_not_free_inverters() {
    // basic.toml contains a real NAND2; it must be chosen, not OR2 with free pin inverters.
    let s = run("inputs/nand.dsl", "libs/basic.toml");
    assert!(s.contains("NAND2"), "should use the native NAND2 cell:\n{}", s);
    assert!(!s.contains("OR2"), "must not emulate with OR2 + inverters:\n{}", s);
    assert!(!s.contains("=!"), "no free pin inversion (pin=!signal) allowed:\n{}", s);
    assert!(s.contains("Total cells used : 1"), "still a single cell:\n{}", s);
}

#[test]
fn decode_counts_the_inverter() {
    // !state[3] & state[2] & state[1] & state[0] => AND4 + a real INV = 2 cells.
    let s = run("inputs/decode.dsl", "libs/with_and4.toml");
    assert!(s.contains("Total cells used : 2"), "AND4 + INV = 2 cells:\n{}", s);
    assert!(s.contains("AND4"), "{}", s);
    assert!(s.contains("INV"), "the state[3] inversion must be a real INV cell:\n{}", s);
    assert!(!s.contains("=!"), "no free pin inversion:\n{}", s);
}

#[test]
fn mux_counts_the_inverter() {
    // (s&b) | (!s&a) needs a real INV for !s => 4 cells.
    let s = run("inputs/mux.dsl", "libs/basic.toml");
    assert!(s.contains("Total cells used : 4"), "adds a real INV for !s:\n{}", s);
    assert!(s.contains("INV"), "{}", s);
    assert!(!s.contains("=!"), "no free pin inversion:\n{}", s);
}

#[test]
fn builtin_inverted_input_is_free() {
    // o = !x & y maps to one AND2B1 (built-in inverted pin a); no separate INV.
    let s = run("inputs/andb.dsl", "libs/with_andb.toml");
    assert!(s.contains("AND2B1"), "{}", s);
    assert!(s.contains("Total cells used : 1"), "built-in inversion is free:\n{}", s);
    assert!(!s.contains("INV"), "no separate inverter needed:\n{}", s);
    assert!(!s.contains("=!"), "{}", s);
}

#[test]
fn no_inv_cell_and_inversion_needed_is_an_error() {
    // no_inv.toml has AND2 + OR2 only; !(a&b) cannot be built without an inverter.
    let manifest = env!("CARGO_MANIFEST_DIR");
    let input_text =
        std::fs::read_to_string(format!("{}/tests/fixtures/inputs/nand.dsl", manifest)).unwrap();
    let res = run_pipeline(RunInputs {
        input_path: "tests/fixtures/inputs/nand.dsl".to_string(),
        input_text,
        library_path: "tests/fixtures/libs/no_inv.toml".to_string(),
    });
    assert!(res.is_err(), "missing INV + required inversion should be a mapping error");
}
```

- [ ] **Step 3: Run the behavioral tests to confirm they fail (red)**

Run: `cargo test --test strict_inversion 2>&1 | tail -30`
Expected: FAILS — `nand_uses_native_nand2...` finds `OR2 (a=!a, b=!b)`, `decode/mux` show counts 1/3 and `=!`, `no_inv...` returns `Ok`. (`builtin_inverted_input_is_free` may already pass.)

- [ ] **Step 4: Rename netlist fields and document the signal model**

Replace the struct definitions in `src/mapper/netlist.rs` (lines 4–19) with:

```rust
#[derive(Debug, Clone)]
pub struct CellInstance {
    pub uid: u32,
    pub cell_id: CellId,
    pub aig_node: NodeId,
    /// This instance physically drives the `(aig_node, output_negated)` signal:
    /// its output value == natural_value(aig_node) XOR output_negated.
    pub output_negated: bool,
    /// One entry per cell PIN, indexed by pin position in the library declaration.
    pub pin_inputs: Vec<PinInput>,
}

#[derive(Debug, Clone)]
pub struct PinInput {
    pub leaf: NodeId,
    /// Which polarity of `leaf` this pin consumes. The `(leaf, leaf_negated)` signal
    /// is produced by a real cell instance (or is a positive primary input). It is
    /// never a free inversion applied at the pin.
    pub leaf_negated: bool,
}
```

- [ ] **Step 5: Make `find_inv_cell` public**

In `src/mapper/phase2.rs`, change the function signature (line 121) from:

```rust
fn find_inv_cell(lib: &CellLib) -> Option<u32> {
```

to:

```rust
pub fn find_inv_cell(lib: &CellLib) -> Option<u32> {
```

- [ ] **Step 6: Thread `has_inv` through `map_aig`**

Replace the body of `map_aig` in `src/mapper/mod.rs` (lines 12–17) with:

```rust
pub fn map_aig(aig: &Aig, lib: &CellLib) -> Result<MappedNetlist, OptCellsError> {
    let cuts = enumerate_cuts(aig);
    let idx = NpnLibIndex::build(lib);
    let has_inv = phase2::find_inv_cell(lib).is_some();
    let p1 = phase1::run(aig, &cuts, &idx, has_inv);
    phase2::run(aig, &cuts, lib, &p1)
}
```

- [ ] **Step 7: Charge real INV cells in phase1**

In `src/mapper/phase1.rs`:

a) Change the signature (line 23):

```rust
pub fn run(aig: &Aig, cuts: &[Vec<Cut>], idx: &NpnLibIndex, has_inv: bool) -> Phase1Result {
```

b) Add this helper just above `pub fn run` (after the `const INF` line):

```rust
fn inv_mapping_sentinel() -> InputMapping {
    InputMapping {
        cell_id: crate::frontend::library::CellId(u32::MAX),
        n_inputs: 0,
        pin_perm: [0; 6],
        input_negation: 0,
        output_negation: false,
    }
}
```

c) Replace the `PrimaryInput` arm (lines 37–41) with a version where the complement is a real `INV`:

```rust
            NodeKind::PrimaryInput { .. } => {
                cost_pos[idx_n] = 0;
                // A complemented primary input must be produced by a real INV cell.
                if has_inv {
                    cost_neg[idx_n] = 1;
                    best_neg[idx_n] = Some(BestChoice {
                        cost: 1,
                        cut_index: usize::MAX,
                        mapping: inv_mapping_sentinel(),
                        via_inv: true,
                    });
                }
                // else: cost_neg stays INF, best_neg stays None (complement infeasible)
                continue;
            }
```

(The `Const0` arm is unchanged: a constant's complement is the opposite rail, not an inverter.)

d) Gate the "positive form + INV" route (lines 95–112) on `has_inv` and reuse the helper:

```rust
        // Consider "compute positive + INV" route for negative — only if an INV cell exists.
        if has_inv && cost_pos[idx_n] < INF {
            let inv_cost = cost_pos[idx_n].saturating_add(1);
            if inv_cost < cost_neg[idx_n] {
                cost_neg[idx_n] = inv_cost;
                best_neg[idx_n] = Some(BestChoice {
                    cost: inv_cost,
                    cut_index: usize::MAX,
                    mapping: inv_mapping_sentinel(),
                    via_inv: true,
                });
            }
        }
```

- [ ] **Step 8: Rewrite the phase2 commit loop**

In `src/mapper/phase2.rs`, replace the entire `while let Some(...)` loop (lines 28–105) with:

```rust
    while let Some((node, neg)) = queue.pop_front() {
        if required.contains_key(&(node, neg)) {
            continue;
        }

        match aig.node(node).kind {
            // Constants are free wires in both polarities (opposite power rail, no cell).
            NodeKind::Const0 => {
                required.insert((node, neg), None);
                continue;
            }
            NodeKind::PrimaryInput { .. } => {
                if !neg {
                    // Positive primary input: external wire, no cell.
                    required.insert((node, false), None);
                    continue;
                }
                // Negated primary input: fall through to best_neg (a real INV cell).
            }
            NodeKind::And2 { .. } => {}
        }

        let choice = if neg {
            p1.best_neg[node.0 as usize].as_ref()
        } else {
            p1.best_pos[node.0 as usize].as_ref()
        }
        .ok_or_else(|| OptCellsError::Mapping {
            message: format!(
                "cannot implement {} polarity of AIG node {}: no matching cell and no inverter available",
                if neg { "negative" } else { "positive" },
                node.0
            ),
        })?;

        if choice.via_inv {
            let inv_id = inv_cell_id.ok_or_else(|| OptCellsError::Mapping {
                message: format!(
                    "AIG node {} requires inversion but library has no INV-class cell",
                    node.0
                ),
            })?;
            let uid = uid_counter;
            uid_counter += 1;
            cells.push(CellInstance {
                uid,
                cell_id: CellId(inv_id),
                aig_node: node,
                output_negated: true,
                pin_inputs: vec![PinInput {
                    leaf: node,
                    leaf_negated: false,
                }],
            });
            required.insert((node, neg), Some(uid));
            queue.push_back((node, false));
        } else {
            let cut = &cuts[node.0 as usize][choice.cut_index];
            let uid = uid_counter;
            uid_counter += 1;
            let n_inputs = choice.mapping.n_inputs as usize;
            let mut pin_inputs: Vec<PinInput> = vec![
                PinInput {
                    leaf: NodeId(0),
                    leaf_negated: false
                };
                n_inputs
            ];
            for (leaf_pos, &leaf) in cut.leaves.iter().enumerate() {
                let pin = choice.mapping.pin_perm[leaf_pos] as usize;
                let leaf_negated = (choice.mapping.input_negation >> leaf_pos) & 1 == 1;
                pin_inputs[pin] = PinInput { leaf, leaf_negated };
            }
            cells.push(CellInstance {
                uid,
                cell_id: choice.mapping.cell_id,
                aig_node: node,
                output_negated: neg,
                pin_inputs,
            });
            required.insert((node, neg), Some(uid));
            for (leaf_pos, &leaf) in cut.leaves.iter().enumerate() {
                let leaf_negated = (choice.mapping.input_negation >> leaf_pos) & 1 == 1;
                queue.push_back((leaf, leaf_negated));
            }
        }
    }
```

- [ ] **Step 9: Rewrite report rendering to resolve produced signals**

In `src/report/mod.rs`, replace the block from `// Resolve PO names per AIG node.` through the end of the cell-rendering `for` loop (lines 36–100) with:

```rust
    // Primary outputs keyed by the (node, polarity) signal they consume.
    let mut po_at: HashMap<(NodeId, bool), Vec<String>> = HashMap::new();
    for (name, node, invert) in &r.netlist.outputs {
        let label = r
            .display_names
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.clone());
        po_at.entry((*node, *invert)).or_default().push(label);
    }

    // Each produced signal (node, polarity) -> the uid of the cell that drives it.
    let mut signal_uid: HashMap<(NodeId, bool), u32> = HashMap::new();
    for c in &r.netlist.cells {
        signal_uid.insert((c.aig_node, c.output_negated), c.uid);
    }

    for c in &r.netlist.cells {
        let cell = r.lib.cells.iter().find(|x| x.id == c.cell_id);
        let cell_name = cell.map(|x| x.name.as_str()).unwrap_or("???");
        let mut pin_str = String::from("(");
        if let Some(cell) = cell {
            for (i, pi) in c.pin_inputs.iter().enumerate() {
                if i > 0 {
                    pin_str.push_str(", ");
                }
                let pin_name = cell.inputs.get(i).map(|s| s.as_str()).unwrap_or("?");
                let label = leaf_label(r, pi.leaf, pi.leaf_negated, &signal_uid);
                pin_str.push_str(&format!("{}={}", pin_name, label));
            }
        }
        pin_str.push(')');
        // A cell drives the (aig_node, output_negated) signal; if a PO consumes exactly
        // that signal, name it after the PO, otherwise after the internal node.
        let out_label = if let Some(names) = po_at.get(&(c.aig_node, c.output_negated)) {
            names.join(", ")
        } else {
            format!("n{}", c.aig_node.0)
        };
        out.push_str(&format!(
            "  u{} : {}  {}  -> {}\n",
            c.uid, cell_name, pin_str, out_label
        ));
    }
```

Then replace the `leaf_label` function (lines 123–139) with:

```rust
fn leaf_label(
    r: &ReportInput,
    leaf: NodeId,
    leaf_negated: bool,
    signal_uid: &HashMap<(NodeId, bool), u32>,
) -> String {
    match &r.aig.node(leaf).kind {
        NodeKind::Const0 => {
            if leaf_negated {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        NodeKind::PrimaryInput { name } => {
            if !leaf_negated {
                r.display_names
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone())
            } else {
                // A complemented primary input is driven by a real INV cell.
                match signal_uid.get(&(leaf, true)) {
                    Some(uid) => format!("n{}_u{}", leaf.0, uid),
                    None => format!("n{}", leaf.0),
                }
            }
        }
        NodeKind::And2 { .. } => match signal_uid.get(&(leaf, leaf_negated)) {
            Some(uid) => format!("n{}_u{}", leaf.0, uid),
            None => format!("n{}", leaf.0),
        },
    }
}
```

- [ ] **Step 10: Rewrite the test simulator to evaluate by (node, polarity)**

In `tests/common/mod.rs`, replace the entire `simulate_netlist` function (lines 34–80) with:

```rust
/// Simulate the MappedNetlist. Each cell drives the signal (aig_node, output_negated);
/// a pin consumes the signal (leaf, leaf_negated). Real INV cells are evaluated like any
/// other cell — there are no free pin inversions.
#[allow(dead_code)]
pub fn simulate_netlist(
    aig: &Aig,
    lib: &CellLib,
    netlist: &MappedNetlist,
    inputs: &HashMap<String, bool>,
) -> HashMap<String, bool> {
    let mut sig: HashMap<(NodeId, bool), bool> = HashMap::new();

    // Seed external signals: positive primary inputs and both polarities of constants.
    for (i, node) in aig.nodes().iter().enumerate() {
        let id = NodeId(i as u32);
        match &node.kind {
            NodeKind::PrimaryInput { name } => {
                sig.insert((id, false), *inputs.get(name).unwrap_or(&false));
            }
            NodeKind::Const0 => {
                sig.insert((id, false), false);
                sig.insert((id, true), true);
            }
            _ => {}
        }
    }

    // Fixpoint: fill a cell's output once all its pin sources are known. DAG => terminates.
    let mut progress = true;
    while progress {
        progress = false;
        for c in &netlist.cells {
            let key = (c.aig_node, c.output_negated);
            if sig.contains_key(&key) {
                continue;
            }
            let mut pattern: u32 = 0;
            let mut ready = true;
            for (pin_idx, pi) in c.pin_inputs.iter().enumerate() {
                match sig.get(&(pi.leaf, pi.leaf_negated)) {
                    Some(true) => pattern |= 1u32 << pin_idx,
                    Some(false) => {}
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if !ready {
                continue;
            }
            let cell_decl = lib
                .cells
                .iter()
                .find(|x| x.id == c.cell_id)
                .expect("cell decl");
            let out = (cell_decl.tt.0 >> pattern) & 1 == 1;
            sig.insert(key, out);
            progress = true;
        }
    }

    let mut out_vals: HashMap<String, bool> = HashMap::new();
    for (name, node, invert) in &netlist.outputs {
        let v = *sig.get(&(*node, *invert)).unwrap_or(&false);
        out_vals.insert(name.clone(), v);
    }
    out_vals
}
```

- [ ] **Step 11: Update the in-crate unit tests for the new signatures/fields**

a) `src/mapper/phase1.rs` test `nand_example_phase1_cost_is_1` — `make_nand2_lib` has no INV, so pass `false`. Change the `run` call (around line 149):

```rust
        let res = run(&aig, &cuts, &idx, false);
```

b) `src/mapper/phase2.rs` test `nand_maps_to_single_nand2_cell` — `nand2_lib` has an INV, so pass `true`. Change the phase1 call (around line 171):

```rust
        let p1 = phase1::run(&aig, &cuts, &idx, true);
```

c) `src/report/mod.rs` test `renders_nand_example` — update the constructed netlist to the new field names. Change the `pin_inputs`/instance fields (lines 172–183) to:

```rust
                pin_inputs: vec![
                    PinInput {
                        leaf: a.node,
                        leaf_negated: false,
                    },
                    PinInput {
                        leaf: b.node,
                        leaf_negated: false,
                    },
                ],
                output_negated: true,
```

- [ ] **Step 12: Build and run the unit + behavioral tests**

Run: `cargo build 2>&1 | tail -5`
Expected: compiles cleanly.

Run: `cargo test --lib --test strict_inversion 2>&1 | tail -30`
Expected: PASS — all five `strict_inversion` tests green; all `--lib` unit tests green.

- [ ] **Step 13: Regenerate and verify the goldens**

Run: `UPDATE_GOLDEN=1 cargo test --test golden 2>&1 | tail -5`
Then inspect the regenerated files:

Run: `cat tests/fixtures/expected/nand_basic.txt tests/fixtures/expected/decode_and4.txt tests/fixtures/expected/mux_basic.txt tests/fixtures/expected/nand_and2_only.txt tests/fixtures/expected/shared_basic.txt`

Verify against the spec's acceptance anchors:
- `nand_basic.txt`: one line `u0 : NAND2  (a=a, b=b)  -> y`; `Total cells used : 1`; `NAND2 × 1`; **no `=!`**.
- `decode_and4.txt`: `Total cells used : 2`; an `AND4` line and an `INV` line; the AND4's state[3] pin references the INV's output (not `!state[3]`); `AND4 × 1` and `INV × 1`.
- `mux_basic.txt`: `Total cells used : 4`; includes one `INV` (for `!s`); `AND2 × 2`, `INV × 1`, `OR2 × 1`; **no `=!`**.
- `nand_and2_only.txt`: still `Total cells used : 2` (`AND2 × 1`, `INV × 1`); the AND2 line now ends `-> n3` (no longer `-> !y`).
- `shared_basic.txt`: unchanged (`Total cells used : 2`).

If any regenerated file shows a `=!` on a pin, the fix is incomplete — re-check Steps 8–9.

- [ ] **Step 14: Run the full test suite**

Run: `cargo test 2>&1 | tail -30`
Expected: PASS — golden (5), strict_inversion (5), proptest (3: functional equivalence, NPN idempotence, monotonicity), and all unit tests green.

Run: `cargo fmt --check && cargo clippy -- -D warnings 2>&1 | tail -5`
Expected: clean (CI parity).

- [ ] **Step 15: Commit**

```bash
git add src/mapper tests/strict_inversion.rs tests/common/mod.rs tests/fixtures src/report
git commit -m "$(cat <<'EOF'
Charge real INV cells for added inversions; no free input-pin inversion

Input-pin inversion is no longer free. The NPN matcher still finds which
leaves a cell would need complemented, but each such complement now costs a
real, counted INV cell (cost_neg for primary inputs is an INV, not 0), and
phase2 materializes it as a cell instance wired to the consumer. The report
identifies signals by (node, polarity) and never prints pin=!signal. Cell
built-in inversions (e.g. AND2B1, INR4) remain free via permutation matching.

- y=!(a&b) + basic.toml now maps to NAND2 (not OR2 with free inverters)
- decode (state==4'b0111) is AND4 + INV = 2 cells
- mux gains a real INV for !s = 4 cells
- missing INV + required inversion is now a mapping error

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Correct the design documentation

**Files:**
- Modify: `docs/superpowers/specs/2026-05-21-opt-cells-design.md`

- [ ] **Step 1: Rewrite §6.4's cost-model assumption**

In `docs/superpowers/specs/2026-05-21-opt-cells-design.md`, replace the "Cost model assumption (important)" subsection (the paragraph beginning **"Input pin inversion is free."** through the `INV cells are counted only when...` line) with:

```markdown
#### Cost model assumption (important)

**Input pin inversion is NOT free.** Two kinds of inversion are treated differently:

- **Built-in inversions** baked into a cell's own `function` (e.g. TSMC `INR4D0BWP7T40P140`'s
  `out = !(!A1 | B1 | B2 | B3)`, where `A1` is inverted inside the cell) are part of the cell's
  truth table and are matched via input **permutation**. They cost nothing extra — the cell
  physically realizes them.
- **Added inversions** — feeding the complement of a signal into a pin the cell does not natively
  invert — must be realized by a real, counted `INV` cell (or by a different cell that natively
  produces the complemented function, e.g. a `NAND` for `!(a&b)`). A complemented primary input
  costs an `INV`. If an inversion is required and the library has no `INV` cell (and no
  native-complement cell), the mapping is infeasible and the tool reports an error.

NPN matching still enumerates which leaves a given cell would need complemented; the cost model
charges each such complement as a real inverter, so the DP prefers the cell that needs the fewest
added inverters (it picks `NAND2` over `OR2` + 2×`INV`, and `INR4` over `AND4` + 3×`INV`).
Inverters are shared across fanout via the per-`(node, polarity)` commit set.
```

- [ ] **Step 2: Rewrite §7.3's inversion formatting rule**

Replace the first bullet of §7.3 ("Input inversion shown inline as `pin=!signal` ...") with:

```markdown
- Input pins render as the driver's signal name (a primary-input name or a producing cell's `uX`
  output); a `!` is never printed on an input pin. An added inversion appears as its own `INV`
  cell line (`uX : INV (a=...) -> ...`) and is included in the "Cell usage" tally. Cells with
  built-in inverted inputs (e.g. `AND2B1`, `INR`-style) absorb their inversion into the cell and
  add no `INV`.
```

Also update the §7.2 decode example netlist line so it no longer shows `a=!state[3]` as a single cell; note that decode maps to `AND4` plus a real `INV` on `state[3]` (2 cells).

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/specs/2026-05-21-opt-cells-design.md
git commit -m "$(cat <<'EOF'
Docs: correct §6.4/§7.3 — input-pin inversion is not free

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Self-Review

**1. Spec coverage** — every spec section maps to a task:
- §2 corrected model (built-in free, added inversions = real INV) → Task 1 Steps 7–9; proven by `builtin_inverted_input_is_free` and `nand_uses_native_nand2...`.
- §3.1 phase1 cost (PI complement, INV gating) → Step 7.
- §3.2 netlist representation → Step 4.
- §3.3 phase2 materialization → Steps 5, 6, 8.
- §3.4 report rendering → Step 9.
- §3.5 matcher unchanged → no task touches `match_npn` (intentional).
- §3.6 feasibility error → Step 8 + `no_inv_cell_and_inversion_needed_is_an_error`.
- §4 acceptance anchors → Steps 2 (behavioral) and 13 (goldens).
- §5 testing (TDD, simulator, proptests) → Steps 2–3, 10, 14.
- §6 doc updates → Task 2.

**2. Placeholder scan** — every code step contains complete code; every run step has a command and expected result. Goldens for `decode`/`mux` are regenerated (Step 13) with explicit verification criteria rather than hand-transcribed node-id labels, because exact internal `nX_uY` labels are output-determined; the meaningful properties (counts, INV presence, absence of `=!`) are locked by the behavioral tests in Step 2.

**3. Type consistency** — `output_negated` / `leaf_negated` are defined in Step 4 and used identically in phase2 (Step 8), report (Step 9), the simulator (Step 10), and the report unit test (Step 11c). `phase1::run(.., has_inv: bool)` (Step 7a) matches all call sites: `map_aig` (Step 6), phase1 test (Step 11a), phase2 test (Step 11b). `phase2::find_inv_cell` is made `pub` (Step 5) before `map_aig` calls it (Step 6). The `inv_mapping_sentinel()` helper (Step 7b) is used in both phase1 edits (Step 7c, 7d).
