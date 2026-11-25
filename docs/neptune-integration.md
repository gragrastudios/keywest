# Neptune renderer integration

KeyWest integrates the renderer, shader translator, streaming, and performance changes
from the user-supplied **Neptune VE 2026-11-25c** source distribution. Both projects
are GPL-3.0-or-later. Original notices remain in the imported source. The distribution's
[notes and credits](neptune-source-notes.txt) are preserved for provenance; their
performance figures describe the author's machine, not measurements of this integration.

Source archive SHA-256:
`fcbc4360bbfb95724196c53016b5448371db5b21d5abf5daf0936921a57ca8f0`.

Integrated changes include paired wave64 execution on wave32 NVIDIA GPUs, descriptor
records copied as bytes, texture mip residency and replacement handling, guarded reads
into title memory, indirect rendering, cached translations, frame-time recording, and
the PC FidelityFX redirect for Series S upscaling. The temporary ambient-light pass
makes unlit interiors visible; it does not implement the title's ray-traced lighting.
Set `VICTORY_AMBIENT=off` to disable it.

KeyWest retains early graphics initialization, automatic mesh-capable adapter selection
with WARP fallback, and its dynamic buffer-address metadata and full wave32 ballot fixes.
Neptune's display-overlay pass replaces the earlier standalone compositor. Captures
must read the composed back buffer so startup UI appears in saved frames too.

The executable, archives, game files, shader packs, proprietary upscaler DLLs, and
Discord integration are not imported. Shader translations are produced locally from
the user's own game files. The existing cache and save locations remain unchanged.

## Running

Build with `cargo build --release -p victory`, then:

```powershell
.\tools\start-keywest.ps1 -Game 'C:\path\to\game_scarlett_master.exe' -FrametimeDump
```

The launcher defaults to the game's new Documents location. `-DryRun` prints the
configuration without opening game files or starting the runtime.
For Series S's lower rendering resolution, supply your own PC upscaler:

```powershell
.\tools\start-keywest.ps1 -FsrDll 'C:\path\to\amd_fidelityfx_upscaler_dx12.dll' -Warm -FrametimeDump
```

Without that DLL the automatic profile selects Series X. `-VramGB 8` or `12` reduces
buffer and staging budgets. Command-list retention is capped below Neptune's original
64 GB RAM profile. `-AutoAdvance` optionally taps A through the startup screens.

The performance switches are opt-in through the launcher. Frame generation is not
configured. The frame-time CSV goes to the configured cache folder as `frametime.csv`.
Compare the same scene, console profile, and warmed shader cache when measuring changes.

## Validation

Source builds and synthetic tests can run without the game. Live startup, world rendering,
and FPS must be checked after the user's game-directory move finishes. No improvement
on the user's machine is claimed until those measurements are available.
