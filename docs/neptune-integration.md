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

Read-only loose windows now use pooled uploads instead of keeping a CPU twin for
each GPU buffer. A window's first declared shader write creates its CPU twin and
switches to the regular merge path, preserving the buffer and its GPU address.
The transition serializes with refreshes, invalidates the page journal, and replaces
queued chunks with a complete baseline upload so old chunks cannot undo it.
This corrects the write loss that previously required disabling transient windows.
Use `-KeepWindowCpuCopies` to compare with the previous profile. Rebuild the runtime
before using the updated launcher: older runtimes do not implement this transition.

The live run before this change held about 2.4 GiB in loose-window CPU twins.
This identifies avoidable allocations, not a measured saving: writable windows and
the shared upload pool still consume RAM. The new transition passes a GPU transfer
and read-back regression with concurrent promotion and a later CPU update. The
default GPU suite passes 183 tests (3 ignored), and strict GPU Clippy passes.
Whole-process savings and the remaining black water patches still need validation.

### Follow-up on NVIDIA, November 26

A five-minute native run of `93d1969` reached the world from a copy of the user's
latest test saves. Loose-window CPU storage ranged from 4 MiB to about 252 MiB
after four windows acquired CPU twins; the shared upload pool reached 512 MiB.
The earlier run retained about 2.4 GiB in loose-window CPU twins. These are
allocation-component observations across different scenes, not a controlled
whole-process saving. Private commit still peaked at about 21.4 GiB. The outdoor
scene's last intervals were about 10 FPS; black patches remained visible.

The portable shader corrections are adapted from Belobug's
[quad-helper change 03629f3](https://github.com/Belobug/VIctory-Belobug-AMD/commit/03629f3f0d3f3329052e05084440aeee75ce3107)
and [first-lane helper correction f520772](https://github.com/Belobug/VIctory-Belobug-AMD/commit/f520772bd1675f7d4bc6397bdb84d5ddfb6b6d76),
under GPL-3.0. Pixel quad permutations use quad operations that include uncovered
helpers, and a pixel helper uses its own value for a first-lane read so descriptor
loops can finish. Compute and mesh wave handling is preserved. A synthetic triangle
test checks both quad instruction forms at uncovered neighbours on NVIDIA and WARP.
That test also passes with the previous implementation on these adapters, so it
does not identify these operations as the cause of the observed black patches.

Repeated shader-write declarations and unaligned constant-buffer scratch fills
are cached until a command-list operation writes title memory. These are adapted
from romainhedouin's Belobug changes, via avatarmanz's
[b32fb99](https://github.com/avatarmanz/VIctory-7900XTX/commit/b32fb9975d74600ac0812fd9424fa2d6829a004a)
and [c96525d](https://github.com/avatarmanz/VIctory-7900XTX/commit/c96525dbb29daa12bc224813a3e64745520fae29),
under GPL-3.0. The adaptation retains Neptune's constant-buffer wait policy and
shader-written range tracking. Regression tests check two different constant
snapshots around a title write and that shader-write declarations resume afterward.
These reduce repeated work; an FPS improvement must be measured separately.
The combined changes pass the default workspace's 625 tests (10 intentionally
ignored) and strict all-features workspace Clippy.

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
transient-window write loss addressed above.

After a restart, a live test of runtime commit `6c79605` reached Chapter 1 and
user-controlled boat driving without the immediate mission failure recurring.
Ocean texture read-back was active at roughly 13 copies per frame. The profile was
Series S, a 16 GB GPU buffer budget, and `-DropMips 1`. Four focused ocean, retained-mip,
and buffer-lifetime regression tests also passed after the restart.

The logged loading peaks were 16.3 GiB resident RAM and 22.7 GiB private commit;
the observed boat view settled around 14 GiB resident and 20 GiB private commit.
The last-minute median of interval FPS samples was 17.8, with hitches (one interval
at 3 FPS). These are observations of one run, not a controlled before/after benchmark.
Water still has visible rectangular patches; memory use remains substantial. Full
mission completion and long-session stability were not established by this check.
