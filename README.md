# desert_rust_metal

The Desert City map drawn with Rust and Metal and no engine at all. It exists
as the control case: the same shader, the same draw loop and the same map as
`desert_rust_bevy`, differing in exactly one thing — where the draw list comes
from.

| | `desert_rust_metal` | `desert_rust_bevy` |
|---|---|---|
| draw list | `Vec<GltfMesh>` built once at load | snapshot extracted from the ECS every frame |
| engine | none | Bevy |
| command buffers per frame | 1 | 1 (was 4) |

`load_gltf_scene` is the renderer's own pre-ECS path, restored from `f15a3a2^`
in `bevy_fps_demo`'s history and now living in `metal_renderer` so both rigs
share it. Node transforms are baked into world-space vertices at load, so the
model matrix handed to the shader is the identity.

## What it settled

Before the mipmap fix, both Rust rigs sat at ~14 ms per frame. That is the
measurement that ruled the ECS out: an engine-free build was exactly as slow,
so whatever was wrong had to be underneath both. Without this rig the
investigation would have kept blaming the extraction path, which costs 0.4 ms.

It is also the yardstick for everything after. At the time of writing it draws
the map in **3.15 ms of GPU time** against the ECS rig's 4.96 ms — a gap with
no identified cause yet, see that rig's README for what has been ruled out.

## Caveats worth knowing

**Bevy is still linked in.** This rig never constructs an `App` and no Bevy
code runs, but `metal_renderer` depends on Bevy for the snapshot types, so it
is compiled and linked: 40 MB of binary against the Swift rig's 371 KB. It
costs build time and app size, nothing at runtime.

**It ignores `alphaMode`.** `load_gltf_scene` never reads it, so the four
BLEND materials in this map draw as opaque. That is a genuine behavioural
difference from the ECS rig, not just a performance one — worth remembering
before treating the two as identical workloads.

**`next_drawable` is timed outside the encode phase here**, unlike the shared
renderer where it sits inside `render_frame`. Compare the two rigs on total
CPU and on `gpu`, never on their encode columns.

## Running it

```sh
./deploy.sh              # build, install, launch
./deploy.sh -c           # ... and stream [stats]
```

`GFX_NO_MIPS=1` turns the mip chain off, which is how the 5x cost of its
absence was measured. The `GFX_NO_SHADOWS` / `GFX_NO_NORMAL_MAP` /
`GFX_NO_SPECULAR` / `GFX_NO_POINT_LIGHT` / `GFX_FLAT` switches compile the
shared shader without that work, same as in the other rig.
