# Portal generation status

State after the CodFaces reland (smart-portals branch). Ground truth where numbers are
quoted: the official IW `dawnville.map` (197 hand-placed portal brushes,
stripped before each run, coverage measured by 32x32 sampling of each
original's face rect against generated portal AABBs, 16-unit tolerance).

## Where things stand

| path | portals | full | partial | untouched | mean area | center-hit | time |
|---|---|---|---|---|---|---|---|
| plane-scan auto pass | 3441 | 86 | 83 | 28 | 0.61 | 117/197 | ~10 s |
| BSP generator, CodFaces (default) | 2148 | 103 | 88 | 6 | 0.71 | 139/197 | ~8 s |
| BSP generator, BrushScoring (fallback) | 4168 | 86 | 104 | 7 | 0.68 | 125/197 | ~5 s |

training_outside BSP baseline (in-repo map, pinned by
`bsp::tests::training_outside_bsp_baseline`): 97 structural brushes,
633 leaves, 1065 candidates, 140 portals created (216 merged away),
max depth 31.

Both dawnville harnesses are measurable via ignored tests (`dbg_dawnville`,
`dbg_dawnville_bsp`), which need the map on disk (`DAWNVILLE_MAP` env or the
default path). The BSP harness takes `BSP_BRUSH=1` (BrushScoring instead of
the CodFaces default), `BSP_MERGE_GAP=<f32>` and `BSP_MAX_DEPTH=<n>`
overrides, prints the originals' face-area profile and lists every untouched
original with center/extents.

## What landed since `bd466b0`

### 1. CodFaces port finished (was the main-line blocker)
Two stacked defects, not one:
- The face list is now built from clipped hull windings
  (`make_visible_bsp_face_list`, CoD `MakeVisibleBspFaceList`/`visibleHull`):
  each side winding minus the parts buried inside other structural brushes
  (0.1 epsilon, AABB-prefiltered). Fully buried sides yield no faces, so
  coincident interior planes never reach the splitter. Alone this moved
  CodFaces from 0.00 to 0.13 mean.
- Root cause of the sliver recursion, found by tracing deep splits: the
  grid force merges its exact-axial plane into a near-axial face plane
  (`(0.99999994,0,-0)`, d=-2048), the merged plane fails the `== 1.0`
  axial test in the bounds seeding, children inherit identical bounds, the
  grid recomputes the identical line, the split partitions nothing, and the
  same plane is re-selected 140 levels deep. Fix: `PlanePool::find` snaps
  near-axial normals (1e-4) to exact axial planes (q3map `FindFloatPlane`
  behaviour). With the snap every split makes progress (grid splits tighten
  bounds, face splits drop at least the owning face), and the remaining
  depth-200 hits are legitimate grid peeling: dawnville's 77k-unit y extent
  needs ~110 peel levels before face depth. `BSP_MAX_DEPTH=512` was tested
  and gives identical coverage for +1 s, so the cap stays at 200.
- Result: CodFaces 0 full / 6 partial / 191 untouched (mean 0.00) ->
  103 / 88 / 6 (mean 0.71), center-hit 1 -> 139. It now beats
  BrushScoring on full/mean/center with half the portal count, so
  `SplitterSelection::CodFaces` is the default again and BrushScoring is
  the fallback.

### 3. Framing discriminator is edge-weighted
`solid_perimeter_fraction` weights edges by length instead of per-edge
counts, so long wall-span edges outvote clusters of short trim-sliver
edges on fragmented leaf portals. CodFaces untouched 16 -> 6 at the same
mean. Note: the same change is neutral-to-negative for BrushScoring
fragments (full 83 -> 75 at gap 16); gap 32 recovers it (86 fulls), and
CodFaces is the default path, so the tradeoff stands.

### 4. BSP pass is in the UI
The 2D Portals menu has *BSP portals (whole map)* next to the plane-scan
*Auto portals*: undoable action, console report line
(leaves/candidates/max depth).

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

### Second pass: prune (landed)
Emission over-generates (~2.7k/5.2k brushes for 197 originals), so
`prune_placed_portals` runs after the merge: drop slabs fully buried in
solid (face samples solid on both sides; partial burials stay, the
tutorials over-cover on purpose) and collapse twin slabs covering one
opening from parallel planes (same axis, planes within 16, rect overlap
> 0.8, volumes touching, so stacked floors never match; keeps the
larger rect). CodFaces: 2746 -> 2148 portals (-22%) at identical
coverage. BrushScoring: 5215 -> 4168 (-20%), fulls intact (86),
untouched 5 -> 7, mean 0.69 -> 0.68. Debugged one real incident along
the way: the burial step used `Vec::retain`, dropping tracking entries
without recording brush ids (25 phantom brushes); it partitions
manually now. `PRUNE_DEBUG=1` logs every dropped slab with its cause.

### 7. Merge gap tested
BSP default `merge_gap` is now 32.0 (was 16): CodFaces full 101 -> 103,
portals 2921 -> 2746, no downside measured. The plane-scan path still uses
`PORTAL_MERGE_GAP = 16`; retesting the scan at 32 is open.

### 10. Regression baselines (dawnville table above + training_outside test)

## Left to do

### BSP generator (main line)

2. **Split candidate classes (measured, not yet split).** Original face
   areas `[<8k, <32k, <128k, <512k, <2M, >=2M]` are `[81, 12, 30, 39, 21,
   14]`: small wall passages vs large separators. The large low-framing
   candidate population is 867 (263 horizontal, 45 slab-scale) against 6
   real slabs, so a "large + low-framing" rule is ~40:1 junk-to-signal
   without cell information. Deferred until cells exist (see 6).
3. (done above; histogram is still bimodal at 0.0/0.3, which now reads as
   interior-fragment vs boundary-fragment, not a classifier defect).
4. (done above.)
5. (done above.)

### Plane-scan (kept as fallback path)

6. **6 untouched on the BSP route: accept as manual.** All six are huge
   horizontal sky/district separator slabs at z=152 (184x2311 up to
   2048x5148, thickness 8), present as zero-framing candidates but
   dropped by the framing floor. By the tutorials these need
   mapper-designated cells; revisit when cells are inferred.
7. (done for BSP above; plane-scan retest at 32 open.)

### Validation

8. **More maps.** training_outside (BSP baseline pinned) and official
   dawnville (both paths) measured. Still open: re-run decompiled
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
