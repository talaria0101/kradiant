# Portal generation status

State after `bd466b0` (smart-portals branch). Ground truth where numbers are
quoted: the official IW `dawnville.map` (197 hand-placed portal brushes,
stripped before each run, coverage measured by 32x32 sampling of each
original's face rect against generated portal AABBs, 16-unit tolerance).

## Where things stand

| path | portals | full | partial | untouched | mean area | center-hit | time |
|---|---|---|---|---|---|---|---|
| plane-scan auto pass | 3441 | 86 | 83 | 28 | 0.61 | 117/197 | ~10 s |
| BSP generator (default selection) | 6343 created / 3942 after merge | 86 | 103 | 8 | 0.67 | 131/197 | 0.8 s |

Both are measurable via ignored tests (`dbg_dawnville`, `dbg_dawnville_bsp`),
which need the map on disk (`DAWNVILLE_MAP` env or the default path).

## Left to do

### BSP generator (main line)

1. **Finish the CodFaces port.** It is implemented and unit-tested but hits
   sliver recursion on dawnville (max_depth cap 200 fires) and produces
   near-zero coverage there, while the real compiler clearly does not.
   Most likely missing piece: CoD's face list is built from *clipped hull*
   windings (`MakeVisibleBspFaceList`, `visibleHull`) over the structural
   brush set, not raw brush sides. Until this is resolved,
   `SplitterSelection::BrushScoring` stays the default.
2. **Split candidate classes.** Wall passages (doorways/windows) and area
   separators (IW's district/sky slabs) share one framing threshold (0.3)
   and one min-edge floor (8). The tutorials treat them differently; two
   tuned paths should lift both classes.
3. **Framing discriminator.** The solid-perimeter histogram is still
   spread (bimodal at 0.0/0.3 with a thin 1.0 spike). Re-check after 1-2
   land; consider edge-weighting by edge length instead of per-edge count.
4. **Wire the BSP pass into the UI.** Only the plane-scan auto pass is in
   the 2D context menu (*Portals -> Auto portals*). The BSP pass needs the
   same treatment (undoable action + report line).
5. **PortalSide detection.** The active `common/portal` face orientation is
   always `Negative`. IW's originals pick the side facing the larger cell;
   a heuristic (face the open-area leaf) would remove manual flipping.

### Plane-scan (kept as fallback path)

6. **28 untouched on the scan route** shrink to 8 through the BSP; the
   stragglers are hand-placed district/sky separator slabs, which by the
   tutorials need mapper-designated cells. Either accept as manual or
   infer cells later.
7. **Partials (extent mismatch).** Merge gap is 16; test 32 (seals piers
   between neighbouring windows, which the originals also do).

### Validation

8. **More maps.** Only training_outside, decompiled Carentan and official
   dawnville have been run. One indoor-heavy official map would prove the
   thick-wall fix generalizes; re-run Carentan through the BSP path (it
   was only measured through the plane scan).
9. **Compiler-level validation.** Round-trip is parse-level only. Feed a
   generated map to the real compiler (original `q3map.exe` under wine or
   the cod-q3map-decomp port) and confirm the cell/portal counts and a
   clean `-bsp` run.
10. **Regression baselines.** Record training_outside and Carentan numbers
    on the current HEAD so future changes have a comparison point.

### Housekeeping

11. Cell-wall 45-degree corner bevels (the tutorials' manual step) remain a
    known manual case; overlaps are reported to the console.
12. Stale generated output maps in the sandbox workspace
    (`dawnville_portals_*.map`, `cod_map_portals_generated.map`) should be
    pruned or regenerated from HEAD.
