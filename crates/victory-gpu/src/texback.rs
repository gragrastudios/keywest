//! Textures shaders write that the title's own code then reads on the CPU.
//! On the console the CPU and the GPU share memory, so what a compute pass
//! stores into a texture is in title memory for the CPU to read; here it
//! stays in the texture's host copy. The title's ocean is one: its wave
//! passes write a 256x256 height map (R32_FLOAT) and a displacement map
//! (R32G32_FLOAT) that its CPU code samples, which is what boats float on.
//! Without the texels there the physics sees no water: boats sink.
//!
//! On by default; `VICTORY_OCEAN_READBACK=0` switches it off. A list in which
//! shaders write a texture of a kind in `VICTORY_TEXTURE_BACK` (comma-
//! separated `<width>x<height>f<DXGI format>`, by default the kinds the
//! title was seen reading: `256x256f41,256x256f16,64x64f16,128x128f36`)
//! copies it into a read-back buffer of ours when it closes; once the list
//! has run ([`crate::host`]'s `read_backs`, which the relay and the late
//! settles already call for every list) the texels are tiled into the
//! texture's title memory in the console's layout, XOR swizzle modes
//! included (`victory_formats::texture::Layout::for_gpu`).
//!
//! Ported from Geckomacabre/vx-layer (commit eee5667, the copy-back), with
//! the XOR addressing from MiamiGame/XTP (commit 00c60fb), both GPL-3.0.
//!
//! shortcut: by kind, not by texture: textures of those kinds the title
//! never reads are copied back too (about a megabyte per such list). The
//! copy shows what the list's last write left; a title read in between sees
//! the frame before, as it would before the console's GPU has run.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Graphics::Direct3D12::*;

use crate::object::This;

/// Ocean simulation outputs must reach the CPU for buoyancy to work.
/// `VICTORY_OCEAN_READBACK=0` is a diagnostic opt-out.
fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_OCEAN_READBACK").is_none_or(|v| v != "0"))
}

/// The kinds copied back: (width, height, format).
fn kinds() -> &'static [(u64, u32, u32)] {
    static KINDS: OnceLock<Vec<(u64, u32, u32)>> = OnceLock::new();
    KINDS.get_or_init(|| {
        if !on() {
            return Vec::new();
        }
        let kinds = parse_kinds(std::env::var("VICTORY_TEXTURE_BACK").ok().as_deref());
        tracing::info!(?kinds, "textures of these kinds that shaders write are copied back into title memory (VICTORY_OCEAN_READBACK=1)");
        kinds
    })
}

/// The upstream boolean form selects the same simulation maps as an unset list.
fn parse_kinds(value: Option<&str>) -> Vec<(u64, u32, u32)> {
    const DEFAULT: &str = "256x256f41,256x256f16,64x64f16,128x128f36";
    let list = match value.map(str::trim) {
        None | Some("1") => DEFAULT,
        Some(list) => list,
    };
    list.split(',').filter_map(|kind| {
        let (size, format) = kind.trim().split_once('f')?;
        let (w, h) = size.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?, format.parse().ok()?))
    }).collect()
}

/// The tile (swizzle) modes whose textures are copied back: `VICTORY_TEXTURE_BACK_MODES`, a
/// comma list of mode numbers, "0" (linear only) when unset.
fn modes_allowed() -> &'static [u32] {
    static MODES: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    MODES.get_or_init(|| {
        std::env::var("VICTORY_TEXTURE_BACK_MODES")
            .unwrap_or_else(|_| "0".into())
            .split(',')
            .filter_map(|m| m.trim().parse().ok())
            .collect()
    })
}

/// Whether title texture `resource` is of a kind copied back.
pub(crate) fn wanted(resource: This) -> bool {
    let kinds = kinds();
    !kinds.is_empty() && crate::resource::texture_kind(resource).is_some_and(|k| kinds.contains(&k))
}

/// A texture's copy into a read-back buffer, recorded into a list, and where
/// its texels go once the list has run.
pub(crate) struct Back {
    buffer: ID3D12Resource,
    footprints: Vec<D3D12_PLACED_SUBRESOURCE_FOOTPRINT>,
    rows: Vec<u32>,
    mips: u32,
    layout: victory_formats::texture::Layout,
    /// The texture's title memory and its layout's bytes.
    at: usize,
    size: usize,
}

/// Records into `list` the copy of `texture` (title resource `resource`'s
/// host copy, in the copy-source state) into a read-back buffer.
pub(crate) fn record(list: &ID3D12GraphicsCommandList, resource: This, texture: &ID3D12Resource) -> Option<Back> {
    let host = crate::host::host()?;
    let (layout, at, size, mode) = crate::resource::title_texture(resource)?;
    // SAFETY: a live texture.
    let desc = unsafe { texture.GetDesc() };
    if !layout.addressable() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::warn!(mode, format = desc.Format.0, "a texture to copy back has no addressing in its swizzle mode: not copied back"));
        return None;
    }
    // Only the tile modes allowed (`VICTORY_TEXTURE_BACK_MODES`, default "0": linear): the
    // ocean's height maps are linear; copies in modes 2 and 27 turned the water to foam with
    // black patches (2026-11-25), likely the base-swizzle assumption of the XOR addressing.
    if !modes_allowed().contains(&mode) {
        return None;
    }
    arm_log(resource, at, &desc, size, mode);
    let mips = u32::from(desc.MipLevels.max(1));
    let subresources = mips * u32::from(desc.DepthOrArraySize.max(1));
    let n = subresources as usize;
    let mut footprints = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); n];
    let mut rows = vec![0u32; n];
    let mut total = 0u64;
    // SAFETY: out arrays of `n` entries.
    unsafe {
        host.device.GetCopyableFootprints(
            &desc,
            0,
            subresources,
            0,
            Some(footprints.as_mut_ptr()),
            Some(rows.as_mut_ptr()),
            None,
            Some(&mut total),
        )
    };
    let buffer = host.read_back_buffer(total)?;
    for (i, footprint) in footprints.iter().enumerate() {
        let to = D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: a borrowed pointer the location never releases.
            pResource: unsafe { std::mem::transmute_copy(&buffer) },
            Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: *footprint },
        };
        let from = D3D12_TEXTURE_COPY_LOCATION {
            // SAFETY: as above.
            pResource: unsafe { std::mem::transmute_copy(texture) },
            Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
            Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: i as u32 },
        };
        // SAFETY: an open list; the texture is in the copy-source state.
        unsafe { list.CopyTextureRegion(&to, 0, 0, 0, &from, None) };
    }
    Some(Back { buffer, footprints, rows, mips, layout, at, size })
}

/// Logs the first copy of each texture: where it lies, its format and size,
/// and the tile mode its texels go in.
fn arm_log(resource: This, at: usize, desc: &D3D12_RESOURCE_DESC, size: usize, mode: u32) {
    static SEEN: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.len() < 32 && !seen.contains(&at) {
        seen.push(at);
        tracing::info!(
            resource = ?resource,
            address = format_args!("{at:#x}"),
            format = desc.Format.0,
            width = desc.Width,
            height = desc.Height,
            title_bytes = size,
            swizzle_mode = mode,
            "a texture shaders write is armed for copy-back into title memory (VICTORY_OCEAN_READBACK=1)"
        );
    }
}

/// Texture copy-backs landed in title memory since the last frame line.
pub(crate) static LANDED: AtomicU64 = AtomicU64::new(0);

/// Once the list that recorded `back` has run: its texels, tiled into the
/// title's layout, into the texture's title memory.
pub(crate) fn land(back: &Back) {
    let mut tiled = vec![0u8; back.size];
    // Bytes of title memory the subresources do not cover stay as they are.
    crate::host::copy_mapped(tiled.as_mut_ptr(), back.at as *const u8, back.size, false);
    let mut data = std::ptr::null_mut();
    // SAFETY: our read-back buffer, written by the list that has run.
    unsafe {
        if back.buffer.Map(0, None, Some(&mut data)).is_err() || data.is_null() {
            return;
        }
    }
    let mut tiled_ok = true;
    for (i, (footprint, &rows)) in back.footprints.iter().zip(&back.rows).enumerate() {
        let (mip, slice) = (i as u32 % back.mips, i as u32 / back.mips);
        let f = footprint.Footprint;
        let pitch = f.RowPitch as usize;
        let len = pitch * rows as usize * f.Depth as usize;
        // SAFETY: the subresource's part of the mapped buffer.
        let linear = unsafe { std::slice::from_raw_parts(data.cast::<u8>().add(footprint.Offset as usize), len) };
        if let Err(e) = back.layout.tile(&mut tiled, mip, slice, linear, pitch, pitch * rows as usize) {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| tracing::warn!(?e, mip, slice, "a texture copied back could not be tiled"));
            tiled_ok = false;
            break;
        }
    }
    // SAFETY: mapped above.
    unsafe { back.buffer.Unmap(0, None) };
    if tiled_ok {
        crate::host::copy_mapped(back.at as *mut u8, tiled.as_ptr(), back.size, true);
        LANDED.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use victory_formats::texture::{Desc, Dimension, Layout, Swizzle};
    use windows::Win32::Graphics::Dxgi::Common::*;

    #[test]
    fn upstream_boolean_readback_setting_keeps_the_ocean_maps() {
        let expected = vec![(256, 256, 41), (256, 256, 16), (64, 64, 16), (128, 128, 36)];
        assert_eq!(parse_kinds(None), expected);
        assert_eq!(parse_kinds(Some("1")), expected);
        assert_eq!(parse_kinds(Some("128x64f41")), vec![(128, 64, 41)]);
        assert!(parse_kinds(Some("0")).is_empty());
    }

    #[test]
    fn ocean_height_and_displacement_written_on_gpu_reach_cpu_memory() {
        let host = crate::host::host().expect("a graphics adapter for ocean read-back");
        for (format, channels) in [
            (DXGI_FORMAT_R32_FLOAT, 1usize),
            (DXGI_FORMAT_R32G32_FLOAT, 2),
        ] {
            let layout = Layout::new(&Desc {
                dimension: Dimension::Tex2d,
                width: 256,
                height: 256,
                depth: 1,
                mips: 1,
                element_bytes: (channels * 4) as u32,
                block: 1,
                samples: 1,
                swizzle: Swizzle::LINEAR,
            })
            .unwrap();
            let mut title = vec![0x5au8; layout.size as usize];
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: 256,
                Height: 256,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: format,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                ..Default::default()
            };
            // SAFETY: test-owned texture, descriptors, and read-back buffer; GPU work is
            // waited for before land maps the buffer and writes into the live title Vec.
            unsafe {
                let mut texture: Option<ID3D12Resource> = None;
                host.device
                    .CreateCommittedResource(
                        &D3D12_HEAP_PROPERTIES {
                            Type: D3D12_HEAP_TYPE_DEFAULT,
                            ..Default::default()
                        },
                        D3D12_HEAP_FLAG_NONE,
                        &desc,
                        D3D12_RESOURCE_STATE_RENDER_TARGET,
                        None,
                        &mut texture,
                    )
                    .unwrap();
                let texture = texture.unwrap();
                let rtv: ID3D12DescriptorHeap = host
                    .device
                    .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                        Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                        NumDescriptors: 1,
                        ..Default::default()
                    })
                    .unwrap();
                let handle = rtv.GetCPUDescriptorHandleForHeapStart();
                host.device.CreateRenderTargetView(&texture, None, handle);
                let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
                let mut rows = 0;
                let mut total = 0;
                host.device.GetCopyableFootprints(
                    &desc,
                    0,
                    1,
                    0,
                    Some(&mut footprint),
                    Some(&mut rows),
                    None,
                    Some(&mut total),
                );
                let buffer = host.read_back_buffer(total).unwrap();
                let list = host.new_list().unwrap();
                list.list
                    .ClearRenderTargetView(handle, &[1.25, -0.5, 0.0, 0.0], None);
                list.list.ResourceBarrier(&[crate::host::transition(
                    &texture,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    D3D12_RESOURCE_STATE_COPY_SOURCE,
                )]);
                let from = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::transmute_copy(&texture),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    ..Default::default()
                };
                let to = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::transmute_copy(&buffer),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        PlacedFootprint: footprint,
                    },
                };
                list.list.CopyTextureRegion(&to, 0, 0, 0, &from, None);
                list.close();
                host.execute(&[&list]);
                let size = title.len();
                land(&Back {
                    buffer,
                    footprints: vec![footprint],
                    rows: vec![rows],
                    mips: 1,
                    layout,
                    at: title.as_mut_ptr() as usize,
                    size,
                });
            }
            for pixel in title[..256 * 256 * channels * 4].chunks_exact(channels * 4) {
                assert_eq!(f32::from_le_bytes(pixel[..4].try_into().unwrap()), 1.25);
                if channels == 2 {
                    assert_eq!(f32::from_le_bytes(pixel[4..8].try_into().unwrap()), -0.5);
                }
            }
        }
    }
}
