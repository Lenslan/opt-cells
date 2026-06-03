# opt-cells — Strict Cell Usage / No Free Input Inversion — Design Spec

**Date**: 2026-06-02
**Status**: Approved for implementation planning
**Audience**: Implementer of the Rust tool
**Supersedes**: §6.4 (cost-model assumption) and §7.3 (input-inversion rendering) of
`2026-05-21-opt-cells-design.md`

---

## 1. Problem statement

The mapper currently treats **input-pin inversion as free**. Through NPN matching it will take
*any* library cell and pretend its input pins can be arbitrarily inverted at zero cost, counting the
result as a single cell and rendering the inversion as a `pin=!signal` annotation.

This is physically wrong. In a real standard-cell flow you may only use the cells the library
actually provides. A cell's input inversions are part of its fixed physical definition (written into
its `function` string) — you cannot bolt an inverter onto a cell's input pin for free.

### 1.1 Concrete evidence of the bug (current behavior)

- `y = !(a & b)` with `basic.toml` — **which contains a `NAND2` cell** — produces
  `u0 : OR2 (a=!a, b=!b) -> y`, counted as **1 cell**. The tool ignored the real `NAND2` and put
  free inverters on an `OR2`'s pins. This is exactly the "直接用 a|b 这个库" failure:
  `!(a&b)` ≡ `!a | !b`, but realizing it requires a cell whose function *is* `!a|!b` (= `NAND2`),
  not the `a|b` cell with its inputs inverted for free.
- `decoded = (state == 4'b0111)` with `with_and4.toml` produces
  `u0 : AND4 (a=state[0], b=state[1], c=state[2], d=!state[3]) -> decoded`, counted as **1 cell** —
  a free inverter on `AND4`'s pin `d`.
- `y = (s & b) | (!s & a)` with `basic.toml` produces `AND2 (a=!s, b=a)` — a free inverter on `s`.

### 1.2 Why the existing tests did not catch it

The functional-equivalence proptest passes because the bug is **not a functional error** — the
mapped netlist computes the correct boolean function. It is a **cost / physical-realizability
error**: free inverters that do not correspond to real cells. The test simulator
(`tests/common/mod.rs`) faithfully reproduces the free `pin.invert` operation, so equivalence holds
while the cell count is understated.

### 1.3 The motivating real cell (correct interpretation)

TSMC `INR4D0BWP7T40P140` has native function `out = !(!A1 | B1 | B2 | B3)`. The inverter on pin `A1`
is **built into the cell** — part of its truth table. The correct way to make such inversions
available is to put the cell in the library with its real function. Permutation matching against the
cell's full truth table then finds it with **zero added inverters**. What is *not* allowed is taking
a plain `AND4` and inverting a pin to emulate that cell.

---

## 2. Corrected model

Two kinds of inversion, treated differently:

1. **Built-in inversions** — inversions baked into a cell's own `function` (e.g. `INR4`'s `A1`).
   These are captured by the cell's full truth table and matched via input **permutation**. They
   cost nothing extra because the cell physically realizes them. Unchanged.

2. **Added inversions** — feeding the complement of a signal into a pin that the cell does not
   natively invert. These are **not free**. Each one must be realized by a **real, counted `INV`
   cell** from the library (or by a different cell that natively produces the complemented function,
   e.g. a `NAND` used to produce `!(a&b)`). An added inversion that no library cell can provide makes
   that mapping infeasible.

**Decision (approved):** when an added inversion is required and no cell provides it natively, the
tool **inserts an explicit `INV` cell from the library and counts it**. The DP still prefers
cells that match natively (it will choose `NAND2` over `OR2` + 2×`INV`). If no `INV` cell exists in
the library and an inversion is genuinely required, the tool reports a clear mapping error.

This makes minimum-**cell-count** optimization honest: every inverter in the answer is a real cell.

### 2.1 Implementation strategy

Keep the NPN matcher. It already enumerates, for each cut, which input-negation pattern lets a given
cell implement that cut. The fix reinterprets each `input_negation` bit from *"free pin inverter"* to
*"this leaf must be fed in its complemented polarity, which costs a real INV cell"*, and charges for
it in the cost model. The DP then minimizes true cell count.

The rejected alternative — restricting the matcher to permutation + output-negation and re-deriving
input inversions in the DP/cut layer — produces identical results with substantially more code and
is not pursued.

---

## 3. Design changes by module

### 3.1 `mapper/phase1.rs` — cost model

- **Primary-input complement is no longer free.** `cost_neg[PI]` becomes
  `if library has an INV cell { 1 } else { INF }` (was `0`). A complemented PI must be produced by a
  real `INV` cell.
- The **"positive form + INV"** route for `best_neg` is only available when the library has an `INV`
  cell. (Today phase1 adds `+1` unconditionally and relies on phase2 to error; phase1 and phase2 must
  agree on INV availability so cost estimates never assume an inverter that cannot exist.)
- Inverted leaves continue to be charged `cost_neg[leaf]`. For internal nodes this may resolve to a
  native-complement cell (e.g. `NAND`) or to positive-form + `INV`; for PIs it is a real `INV`. No
  path charges `0` for an added inversion.
- **Constants** (`Const0`) keep `cost_pos = cost_neg = 0`. A constant's complement is the opposite
  power rail (a tie), not an inverter; constant handling is out of scope for this fix.

### 3.2 `mapper/netlist.rs` — representation

Make every produced signal correspond to exactly one real cell instance, identified by
`(aig_node, polarity)`. No free per-pin inversion.

```rust
pub struct CellInstance {
    pub uid: u32,
    pub cell_id: CellId,
    pub aig_node: NodeId,
    /// This instance physically drives the (aig_node, output_negated) signal:
    /// its output value == natural_value(aig_node) XOR output_negated.
    pub output_negated: bool,
    pub pin_inputs: Vec<PinInput>,
}

pub struct PinInput {
    pub leaf: NodeId,
    /// Which polarity of `leaf` this pin consumes. The (leaf, leaf_negated) signal
    /// must be produced by some cell instance (or be a positive primary input).
    pub leaf_negated: bool,
}
```

A signal is `(NodeId, bool)`. A positive primary input has no producing cell (external, rendered by
name). A **negated** primary input is produced by an `INV` instance whose `aig_node` is that PI and
`output_negated = true`. Internal nodes produced in both polarities have two instances.

(`output_negated` replaces the old `produces_negation`; `leaf_negated` replaces the old per-pin
`invert` but now denotes a real produced signal, never a free flip.)

### 3.3 `mapper/phase2.rs` — materialization

- Maintain `(NodeId, bool) -> uid` for every required signal.
- When a chosen mapping needs leaf `L` complemented, enqueue `(L, true)` so the `!L` signal is
  produced by a real cell, and wire the consuming pin to that producer. This applies uniformly to
  internal nodes **and primary inputs** (a negated PI yields an `INV` instance).
- The consuming pin records `(leaf = L, leaf_negated = true)` and resolves to the producer's `uid`.
  There is no longer a "free invert" branch.
- `total_cells` counts all instances, INV cells included.
- Inserting an `INV` requires the library to contain one; otherwise return
  `OptCellsError::Mapping { message }` naming the signal/node that needs inversion.

### 3.4 `report/mod.rs` — rendering

- Resolve each pin via `(leaf, leaf_negated) -> producing uid -> signal label`. Positive PIs render
  by (display) name; everything else (negated PIs, internal nodes) renders as the producer's `uX`
  output. **No `!` is ever printed on an input pin.**
- Inverters appear as their own netlist lines (`uX : INV (a=...) -> ...`) and in the "Cell usage"
  tally.
- Primary-output polarity: a PO that needs an inverted signal is driven by a real producing cell
  (an `INV` or a native-complement cell); it is not a free output flip.

### 3.5 `match_npn/*` — unchanged

The canonical-form and index logic stays. Its NPN enumeration (including input-negation matches) is
still correct and is what tells the cost model which leaves a given cell would need complemented.
Existing matcher unit tests remain valid.

### 3.6 `error.rs` — unchanged surface

Reuse `OptCellsError::Mapping { message }` for "inversion required but no `INV` cell available".

---

## 4. Verifiable expected results (acceptance anchors)

| Case | Library | Before (wrong) | After (correct) |
|---|---|---|---|
| `y = !(a & b)` | `basic.toml` (has NAND2) | `OR2 (a=!a, b=!b)` = **1** | `NAND2 (a=a, b=b)` = **1** |
| `decoded = (state==4'b0111)` | `with_and4.toml` | `AND4 (d=!state[3])` = **1** | `AND4` + `INV(state[3])` = **2** |
| `y = (s&b)\|(!s&a)` | `basic.toml` | `AND2 (a=!s, …)` = **3** | + `INV(s)` = **4** |
| `y = !(a & b)` | `and2_only.toml` | `AND2`+`INV` = 2 | **unchanged = 2** |
| `shared.dsl` (two outputs) | `basic.toml` | 2 | **unchanged = 2** |

The first row proves the tool now selects the correct native cell instead of a free-inverter
emulation. The "unchanged" rows confirm the existing output-inversion path (`best_neg` via `INV`) and
inversion-free mappings are preserved.

A new golden test should also cover a cell with a **built-in inverted input** (e.g. add an
`INR`-style cell whose function is `!(!a | b | c | d)`), asserting it matches a suitable cut as **1
cell with no added INV** — locking in that built-in inversions stay free.

---

## 5. Testing strategy

- **TDD ordering:** update golden expected files (`nand_basic`, `decode_and4`, `mux_basic`) and the
  affected unit tests to the corrected expectations *first* (red), then implement until green.
- **Golden tests:** regenerate the three changed expected files; add the built-in-inverted-input
  golden described in §4.
- **Test simulator (`tests/common/mod.rs`):** remove the free `pi.invert` step; simulate `INV`
  instances like any other cell, reading each pin from its producer's output signal under the new
  `(node, polarity)` representation.
- **Property tests (must still pass):**
  - *Functional equivalence* — mapped netlist still equals the AIG on all input assignments.
  - *Monotonicity* — adding a cell never increases cell count.
  - *NPN idempotence* — matcher math unchanged.
- **New unit assertions:**
  - `cost_neg[PI]` is `INF` when the library has no `INV`, `1` when it does.
  - A library with no `INV` and an unavoidable inversion yields a `Mapping` error.
  - No `CellInstance`/report path emits a free input inversion (the `pin=!` form is gone).

---

## 6. Documentation updates

- Rewrite §6.4 ("Cost model assumption") of `2026-05-21-opt-cells-design.md`: replace
  "Input pin inversion is free" with the corrected model — built-in inversions are free (matched via
  the cell's truth table under permutation); added inversions cost a real, counted `INV` cell; the
  `INR4` example illustrates a *built-in* inversion, not a license to invert arbitrary pins.
- Rewrite §7.3: input pins render as driver signal names; inverters appear as explicit `INV` cell
  lines; remove the `pin=!signal` formatting rule.

---

## 7. Out of scope (unchanged from v1)

- Area/delay optimization (still cell-count only).
- Constant/tie-cell modeling (constant complements remain free).
- Any change to the DSL, library TOML schema, AIG construction, or cut enumeration.
- The Phase-1 sharing under-estimate (documented known suboptimality) is not addressed here.
