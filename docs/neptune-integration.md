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

Ocean height/displacement texture read-back is enabled by default. These GPU outputs
are also CPU simulation inputs: without the copy-back, boats can sink and fail their
mission. `VICTORY_OCEAN_READBACK=0` disables it for diagnosis. Copy-back remains limited
to the known simulation texture formats and linear layouts.

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

The launcher finds a unique executable under `Documents\GTA6`. `-DryRun` prints the
configuration without opening game files or starting the runtime.
For Series S's lower rendering resolution, supply your own PC upscaler:

```powershell
.\tools\start-keywest.ps1 -FsrDll 'C:\path\to\amd_fidelityfx_upscaler_dx12.dll' -Warm -FrametimeDump
```

The launcher also checks `%LOCALAPPDATA%\victory\dependencies\fidelityfx` for
`amd_fidelityfx_upscaler_dx12.dll`; an explicit `-FsrDll` takes precedence.
Without that DLL the automatic profile selects Series X. `-VramGB 8` or `12` reduces
buffer and staging budgets. Command-list retention is capped below Neptune's original
64 GB RAM profile. `-AutoAdvance` optionally taps A through the startup screens.

The launcher now defaults to `-DropMips 1`: eligible large, CPU-filled textures omit
their finest mip on the host, reducing texture allocation and upload work at the cost
of texture detail. Render targets, unordered-access simulation textures, small textures,
and short mip chains are excluded. Use `-DropMips 0` for full texture detail, or
`-DropMips 2` to reduce detail further. This is independent of the VRAM buffer budget.
Reduced textures keep their title mip offsets for uploads and remap retained mip
indices for read-back, including array slices. They use committed reduced textures,
while the unchanged textures remain eligible for levels-on-demand streaming.

The mip reduction and retained-mip read-back are adapted from
[ofc0urse's change b4ff304](https://github.com/soldsawn/victory-nvidia-8gb/commit/b4ff304f8ded27ea39f656810e0872a68361e865)
and [Filip722's correction fc4b3dd](https://github.com/soldsawn/victory-nvidia-8gb/commit/fc4b3dd564f0a7c13c8b67a83d31a347b7b4e6f8),
under GPL-3.0-or-later. The staging integration preserves Neptune's prefix reads.

Buffer lookup caches hold weak references so inactive recording threads cannot pin
released GPU copies. Cache expiry responds to both free physical RAM and remaining
Windows commit, and idle command lists are trimmed after five seconds. These changes
reduce avoidable retention; the title's own allocations and live GPU resources remain
necessary and a fixed RAM target is not guaranteed.

Transient window uploads are disabled in the launcher. Live world-entry logs showed
shader writes reaching these windows, which have no CPU copy to merge writes back
into. The regular window path retains that copy. This removes the unsafe profile
setting; it does not establish that all geometry or shading defects are resolved.

The performance switches are opt-in through the launcher. Frame generation is not
configured. The frame-time CSV goes to the configured cache folder as `frametime.csv`.
Compare the same scene, console profile, and warmed shader cache when measuring changes.
For this title, warm-up scans the executable, shader pack, and BVH builder instead of
reading every unrelated game archive.

## Validation

The combined source passes both strict Clippy configurations, the default workspace
tests (620 passed), and the diagnostics workspace tests (633 passed), with 10 tests
intentionally ignored in each run. A final diagnostics GPU run with full-detail textures
passes 195 tests (3 ignored), including the boolean read-back compatibility case.
GPU tests exercise ocean height/displacement landing in CPU memory, retained array mip
read-back, released-copy lifetime, the display overlay, descriptor metadata, and buffer
patches. The remapping test now waits for its
reader thread to start, avoiding a scheduling-dependent false failure under CPU load.

Warm-up on the target NVIDIA wave32 GPU translated 25,758 shaders and created 2,734
compute pipelines with zero refusals. This proves translation and pipeline creation;
it does not prove correct rendering of every scene.

The user subsequently reached the world and reported broken shading and geometry,
followed by upload-buffer and Rust allocation failures. Monitored reproductions
confirmed commit exhaustion during loading/world entry. A separate experiment with
a 3 GiB texture preload cap delayed exhaustion but still required the monitor to stop
the process; that cap is not enabled by the launcher. Live logs also exposed the
transient-window write loss addressed above. A stable world run and the visual effect
of that correction remain unverified; no measured performance improvement is claimed.
