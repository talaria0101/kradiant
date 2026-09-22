# Portal generation status

State after the prune/absorb work (smart-portals branch). Ground truth where numbers are
quoted: the official IW `dawnville.map` (197 hand-placed portal brushes,
stripped before each run, coverage measured by 32x32 sampling of each
original's face rect against generated portal AABBs, 16-unit tolerance).

## Where things stand

| path | portals | full | partial | untouched | mean area | center-hit | time |
|---|---|---|---|---|---|---|---|
| plane-scan auto pass | 3441 | 86 | 83 | 28 | 0.61 | 117/197 | ~10 s |
| BSP generator, BrushScoring (default) | 3858 | 95 | 98 | 4 | 0.72 | 135/197 | ~5 s |
| BSP generator, CodFaces | 2513 | 80 | 103 | 14 | 0.60 | 118/197 | ~8 s |

training_outside BSP baseline (in-repo map, pinned by
`bsp::tests::training_outside_bsp_baseline`): 97 structural brushes,
1118 leaves, 1808 candidates, 168 portals created (302 merged, 25
pruned), max depth 18.

Both dawnville harnesses are measurable via ignored tests (`dbg_dawnville`,
`dbg_dawnville_bsp`), which need the map on disk (`DAWNVILLE_MAP` env or the
default path). The BSP harness takes `BSP_COD=1` (CodFaces instead of the
BrushScoring default), `BSP_MERGE_GAP=<f32>`, `BSP_MAX_DEPTH=<n>` and
`BSP_MAX_UNION=<f32>` overrides, prints the originals' face-area profile
and lists every untouched original with center/extents.

## What landed since `bd466b0`

### 1. CodFaces port: sliver recursion fixed, but BrushScoring stays default
Two stacked defects fixed:
- The face list is built from clipped hull windings
  (`make_visible_bsp_face_list`, CoD `MakeVisibleBspFaceList`/`visibleHull`).
- Root cause of the sliver recursion, found by tracing deep splits: the
  grid force merges its exact-axial plane into a near-axial face plane
  (`(0.99999994,0,-0)`, d=-2048), which fails the `== 1.0` axial test in
  the bounds seeding, so children inherit identical bounds and the same
  plane re-splits nothing 140 levels deep. `PlanePool::find` now snaps
  near-axial normals (1e-4) to exact axial planes.

  With honest measurement (see below) CodFaces reaches 80 full / 14
  untouched (mean 0.60) — better than its old 0/191, but worse than
  BrushScoring's 95/4 (0.72). Reason: the grid force optimizes for
  compilation balance and spends most of its depth budget slicing skybox
  void (dawnville ships 150k-unit structural skybox brushes), while
  brush-face-aligned splits place portals on the architecture. Deeper
  caps (512/1024) add leaves but zero coverage, so depth is not the
  limiter. BrushScoring is the default; CodFaces stays available for
  compiler-coupled placement once cells exist.

### 3. Framing discriminator is edge-weighted
`solid_perimeter_fraction` weights edges by length instead of per-edge
counts, so long wall-span edges outvote clusters of short trim-sliver
edges on fragmented leaf portals.

### 4. BSP pass is in the UI
The 2D Portals menu has *BSP portals (whole map)* next to the plane-scan
*Auto portals*: undoable action, console report line
(created/pruned/leaves/candidates/max depth).

### 5. PortalSide detection
`BspPortalParams::auto_side` (default true) faces each portal toward the
deeper open cell: march up to 2048 units behind both caps through open
leaves (stops at solid/outside), deeper run wins, ties keep Negative.
Pinned by `auto_side_faces_the_larger_room` (asymmetric rooms).
Side-effect find: `portal_brush_from_winding` textured the OPPOSITE cap
from `generate_opening_portal_brush` for the same `PortalSide` (sign error
against its own comment), so every merged BSP portal silently flipped its
facing. Fixed to match; merges keep the majority side by member volume
(`PlacedPortal::side`), ties keep the caller's side.

### Second pass: absorb + burial (landed)
Emission over-generates, so `prune_placed_portals` runs after the merge:
small slabs fold into larger nearby ones (same axis, planes within 24,
volumes touching, small rect inside the large rect dilated by 32), the
keeper expands to the union (coverage only grows); then fully-buried
slabs go. Each portal takes part in at most one absorption (keepers
freeze), so folds stay local and cannot chain across districts; order is
burial-first so expanded keepers never face the burial sampler.
`PRUNE_DEBUG=1` logs every fold/drop with its cause. Debugged two real
incidents along the way: a burial `Vec::retain` that dropped tracking
entries without recording brush ids (25 phantoms; partition manually),
and a twin overlap threshold that stranded a real window (absorption
replaces twins entirely now).

### Skybox detection and where the map ends (landed)
A brush with a sky face and no drawn face at all (`is_skybox_brush`:
dawnville's 12 pure-sky shell brushes, cyt's 6 sky+caulk hull pieces)
is boundary, not structure. Two coupled uses: the tree keeps the full
sealed set (sky planes are load-bearing global splitters; excluding
them cost 24 fulls on dawnville), while emission additionally rejects
portals outside the tight town bounds (skybox excluded) plus a 64-unit
margin. Result on identical coverage: dawnville 4129 -> 3858 portals,
cyt 854 -> 697, zero generated portals outside town+64 on either map.
Brushes mixing sky with drawn faces (rooftop open to sky) stay
structural; pinned by `skybox_shell_is_not_structural`.

### Auto portals menu item removed
The plane-scan whole-map pass (*Portals -> Auto portals*) is out of
the 2D menu; the BSP pass is the whole-map generator. The scan
function stays in code (unit tests, dbg harness) as fallback.

### 7. Merge gap 32; union extents capped
BSP default `merge_gap` is 32.0 (was 16). Unions (merge + absorption)
are capped at `BspPortalParams::max_union` (default 16384): chained
fragments tile void without bound otherwise. Sweep showed the cap value
is coverage-neutral on CodFaces while deleting continent slabs from the
map file. The plane-scan path still uses `PORTAL_MERGE_GAP = 16`
(uncapped); retesting the scan at 32 is open.

### 10. Regression baselines (dawnville table above + training_outside test)

### A note on honest numbers (kernel scale visibility)
Mid-session readings up to 181 fulls were inflated: placed portals were
tracked by kernel-derived AABBs, and the geometry kernel misreads a few
thin dawnville-scale slabs as continent boxes (a 102x21 portal read as
75k units), which the merge then rebuilt for real and which blanketed
the metric. Emission now tracks analytic prism bounds (exact), unions
are capped, and the kernel is pinned exact on 8k/16k boxes at dawnville
coordinates by `kernel_box_scale_limits`. All table numbers above are
post-fix. Residual noise: a degenerate brush can still read large in
coverage while being tight in the map; rare, bounded, noted.

## Left to do

### BSP generator (main line)

2. **Split candidate classes (measured, not yet split).** Original face
   areas `[<8k, <32k, <128k, <512k, <2M, >=2M]` are `[81, 12, 30, 39, 21,
   14]`: small wall passages vs large separators. The large low-framing
   candidate population is 867 (263 horizontal, 45 slab-scale) against a
   handful of real slabs, so a "large + low-framing" rule is ~40:1
   junk-to-signal without cell information. Deferred until cells exist
   (see 6).
3. (done above; the histogram stays bimodal at 0.0/0.3, which reads as
   interior-fragment vs boundary-fragment, not a classifier defect).
4. (done above.)
5. (done above.)

### Plane-scan (kept as fallback path)

6. **4 untouched on the BSP route: accept as manual.** Three are huge
   horizontal sky/district separator slabs at z=152, the fourth a
   2836-long district wall section; all need mapper-designated cells.
   Revisit when cells are inferred.
7. (done for BSP above; plane-scan retest at 32 open.)

### Validation

8. **More maps.** training_outside (BSP baseline pinned) and official
   dawnville (all three paths) measured. Still open: re-run decompiled
   Carentan through the BSP path (only plane-scanned so far) and one
   indoor-heavy official map to prove the fixes generalize.
9. **Compiler-level validation.** Round-trip is parse-level only. Feed a
   generated map to the real compiler (original `q3map.exe` under wine or
   the cod-q3map-decomp port) and confirm the cell/portal counts and a
   clean `-bsp` run.
10. (done above.)

### Housekeeping

11. Cell-wall 45-degree corner bevels (the tutorials' manual step) remain a
    known manual case; overlaps are reported to the console.
12. Stale generated output maps in the sandbox workspace
    (`dawnville_portals_*.map`, `cod_map_portals_generated.map`) should be
    pruned or regenerated from HEAD.
