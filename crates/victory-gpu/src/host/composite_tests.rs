#[cfg(test)]
mod tests {
    use crate::host::*;

    /// Different plane resolutions and translucent UI
    /// must compose into the same image, rather than dropping the UI plane.
    #[test]
    fn scales_and_blends_overlay_on_gpu() {
        let Some(host) = crate::host::host() else {
            return;
        };
        // SAFETY: test-owned resources, descriptors, and a list waited for before readback.
        unsafe {
            let texture = |size, state| {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: size,
                    Height: size as u32,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                    ..Default::default()
                };
                let mut resource: Option<ID3D12Resource> = None;
                host.device
                    .CreateCommittedResource(
                        &D3D12_HEAP_PROPERTIES {
                            Type: D3D12_HEAP_TYPE_DEFAULT,
                            ..Default::default()
                        },
                        D3D12_HEAP_FLAG_NONE,
                        &desc,
                        state,
                        None,
                        &mut resource,
                    )
                    .unwrap();
                resource.unwrap()
            };
            let base = texture(4, D3D12_RESOURCE_STATE_RENDER_TARGET);
            let overlay = texture(8, D3D12_RESOURCE_STATE_RENDER_TARGET);
            let back = texture(4, D3D12_RESOURCE_STATE_COPY_DEST);
            let list = host.new_list().unwrap();
            let views: ID3D12DescriptorHeap = host
                .device
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 2,
                    ..Default::default()
                })
                .unwrap();
            let first = views.GetCPUDescriptorHandleForHeapStart();
            let second = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: first.ptr
                    + host
                        .device
                        .GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV)
                        as usize,
            };
            host.device.CreateRenderTargetView(&base, None, first);
            host.device.CreateRenderTargetView(&overlay, None, second);
            list.list
                .ClearRenderTargetView(first, &[1.0, 0.0, 0.0, 1.0], None);
            list.list
                .ClearRenderTargetView(second, &[0.0, 0.5, 0.0, 0.5], None);
            list.list.ResourceBarrier(&[transition(
                &base,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            )]);
            list.list.CopyResource(&back, &base);
            assert!(host.record_overlay(
                &list.list,
                &back,
                DXGI_FORMAT_R8G8B8A8_UNORM,
                (4, 4),
                &overlay,
                D3D12_RESOURCE_STATE_RENDER_TARGET
            ));
            list.list.ResourceBarrier(&[transition(
                &back,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            )]);
            let read = host.read_back_buffer(1024).unwrap();
            let src = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(&back),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                ..Default::default()
            };
            let dst = D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::transmute_copy(&read),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: 0,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            Width: 4,
                            Height: 4,
                            Depth: 1,
                            RowPitch: 256,
                        },
                    },
                },
            };
            list.list.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
            list.close();
            host.execute(&[&list]);
            let mut data = std::ptr::null_mut();
            read.Map(0, None, Some(&mut data)).unwrap();
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), 1024);
            for y in 0..4 {
                for x in 0..4 {
                    let pixel = &bytes[y * 256 + x * 4..y * 256 + x * 4 + 4];
                    assert!(
                        (126..=128).contains(&pixel[0]) && (127..=129).contains(&pixel[1]),
                        "{pixel:?}"
                    );
                    assert_eq!(&pixel[2..], &[0, 255]);
                }
            }
            read.Unmap(0, None);
        }
    }
}
