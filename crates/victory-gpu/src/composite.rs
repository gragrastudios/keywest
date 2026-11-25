//! Display planes composed in submission order into the desktop back buffer.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::{Result, s};

use crate::host::{Host, HostList, compile, transition};

pub(crate) struct Layer {
    pub texture: ID3D12Resource,
    pub state: u32,
    pub source: Option<[i32; 4]>,
    pub destination: Option<[f32; 4]>,
}

const SHADER: &str = r#"
#define RS "DescriptorTable(SRV(t0)), RootConstants(num32BitConstants=4,b0), StaticSampler(s0,filter=FILTER_MIN_MAG_MIP_LINEAR,addressU=TEXTURE_ADDRESS_CLAMP,addressV=TEXTURE_ADDRESS_CLAMP)"
Texture2D<float4> image : register(t0);
SamplerState linearClamp : register(s0);
cbuffer Source : register(b0) { float4 crop; };
struct Vertex { float4 position : SV_Position; float2 uv : TEXCOORD0; };
[RootSignature(RS)]
Vertex vertex(uint id : SV_VertexID) {
    float2 uv = float2((id << 1) & 2, id & 2);
    Vertex v;
    v.position = float4(uv * float2(2,-2) + float2(-1,1),0,1);
    v.uv = crop.xy + uv * crop.zw;
    return v;
}
[RootSignature(RS)]
float4 pixel(Vertex v) : SV_Target { return image.SampleLevel(linearClamp,v.uv,0); }
"#;

pub(crate) struct Pass {
    root: ID3D12RootSignature,
    pipeline: ID3D12PipelineState,
}

impl Pass {
    pub fn new(device: &ID3D12Device, format: DXGI_FORMAT) -> Result<Self> {
        let vs = compile(SHADER, s!("vertex"), s!("vs_5_1"))?;
        let ps = compile(SHADER, s!("pixel"), s!("ps_5_1"))?;
        let code = |b: &[u8]| D3D12_SHADER_BYTECODE {
            pShaderBytecode: b.as_ptr().cast(),
            BytecodeLength: b.len(),
        };
        // SAFETY: shader bytecode contains the root signature; all pointers live through creation.
        unsafe {
            let root = device.CreateRootSignature(0, &vs)?;
            let mut desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                pRootSignature: std::mem::transmute_copy(&root),
                VS: code(&vs),
                PS: code(&ps),
                SampleMask: u32::MAX,
                RasterizerState: D3D12_RASTERIZER_DESC {
                    FillMode: D3D12_FILL_MODE_SOLID,
                    CullMode: D3D12_CULL_MODE_NONE,
                    DepthClipEnable: true.into(),
                    ..Default::default()
                },
                PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                NumRenderTargets: 1,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                ..Default::default()
            };
            desc.RTVFormats[0] = format;
            desc.BlendState.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D12_BLEND_ONE,
                DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D12_BLEND_OP_ADD,
                SrcBlendAlpha: D3D12_BLEND_ONE,
                DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D12_BLEND_OP_ADD,
                LogicOp: D3D12_LOGIC_OP_NOOP,
                RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
                ..Default::default()
            };
            let pipeline = device.CreateGraphicsPipelineState(&desc)?;
            Ok(Self { root, pipeline })
        }
    }

    /// Records premultiplied alpha composition; inputs return to their original states.
    /// The output starts in COPY_DEST and finishes in COPY_SOURCE for capture/presentation.
    pub fn record(
        &self,
        host: &Host,
        copy: &HostList,
        back: &ID3D12Resource,
        layers: &[Layer],
    ) -> Result<()> {
        // SAFETY: live resources and an open command list. Descriptors and inputs are kept
        // by this list through completion, including when presents are sent ahead.
        unsafe {
            let device = &host.device;
            copy.keep(&self.root);
            copy.keep(&self.pipeline);
            let views: ID3D12DescriptorHeap =
                device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    NumDescriptors: layers.len() as u32,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    ..Default::default()
                })?;
            let target: ID3D12DescriptorHeap =
                device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 1,
                    ..Default::default()
                })?;
            copy.keep(&views);
            copy.keep(&target);
            copy.keep(back);
            let output = back.GetDesc();
            let rtv = target.GetCPUDescriptorHandleForHeapStart();
            device.CreateRenderTargetView(back, None, rtv);
            let list = &copy.list;
            list.ResourceBarrier(&[transition(
                back,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            )]);
            list.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 1.0], None);
            list.OMSetRenderTargets(1, Some(&rtv), false, None);
            list.SetDescriptorHeaps(&[Some(views.clone())]);
            list.SetGraphicsRootSignature(&self.root);
            list.SetPipelineState(&self.pipeline);
            list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            list.RSSetScissorRects(&[RECT {
                left: 0,
                top: 0,
                right: output.Width as i32,
                bottom: output.Height as i32,
            }]);
            let stride =
                device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
            for (i, layer) in layers.iter().enumerate() {
                let desc = layer.texture.GetDesc();
                let Some(format) = crate::host::swap_format(desc.Format) else {
                    continue;
                };
                let view = D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: format,
                    ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                        Texture2D: D3D12_TEX2D_SRV {
                            MipLevels: 1,
                            ..Default::default()
                        },
                    },
                };
                device.CreateShaderResourceView(
                    &layer.texture,
                    Some(&view),
                    D3D12_CPU_DESCRIPTOR_HANDLE {
                        ptr: views.GetCPUDescriptorHandleForHeapStart().ptr + i * stride as usize,
                    },
                );
                let state = D3D12_RESOURCE_STATES(layer.state as i32);
                if state != D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE {
                    list.ResourceBarrier(&[transition(
                        &layer.texture,
                        state,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                    )]);
                }
                let (crop, viewport) = rectangles(
                    layer,
                    desc.Width as f32,
                    desc.Height as f32,
                    output.Width as f32,
                    output.Height as f32,
                );
                list.RSSetViewports(&[viewport]);
                list.SetGraphicsRoot32BitConstants(1, 4, crop.as_ptr().cast(), 0);
                list.SetGraphicsRootDescriptorTable(
                    0,
                    D3D12_GPU_DESCRIPTOR_HANDLE {
                        ptr: views.GetGPUDescriptorHandleForHeapStart().ptr
                            + i as u64 * u64::from(stride),
                    },
                );
                list.DrawInstanced(3, 1, 0, 0);
                if state != D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE {
                    list.ResourceBarrier(&[transition(
                        &layer.texture,
                        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                        state,
                    )]);
                }
                copy.keep(&layer.texture);
            }
            list.ResourceBarrier(&[transition(
                back,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            )]);
        }
        Ok(())
    }
}

fn rectangles(
    layer: &Layer,
    width: f32,
    height: f32,
    output_width: f32,
    output_height: f32,
) -> ([f32; 4], D3D12_VIEWPORT) {
    let [left, top, right, bottom] = layer.source.unwrap_or([0, 0, width as i32, height as i32]);
    let crop = [
        left as f32 / width,
        top as f32 / height,
        (right - left) as f32 / width,
        (bottom - top) as f32 / height,
    ];
    let [left, top, right, bottom] = layer.destination.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    (
        crop,
        D3D12_VIEWPORT {
            TopLeftX: left * output_width,
            TopLeftY: top * output_height,
            Width: (right - left) * output_width,
            Height: (bottom - top) * output_height,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Different plane resolutions, normalized placement, and translucent UI
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
            let layers = [
                Layer {
                    texture: base,
                    state: D3D12_RESOURCE_STATE_RENDER_TARGET.0 as u32,
                    source: None,
                    destination: None,
                },
                Layer {
                    texture: overlay,
                    state: D3D12_RESOURCE_STATE_RENDER_TARGET.0 as u32,
                    source: Some([2, 2, 6, 6]),
                    destination: Some([0.5, 0.0, 1.0, 1.0]),
                },
            ];
            Pass::new(&host.device, DXGI_FORMAT_R8G8B8A8_UNORM)
                .unwrap()
                .record(host, &list, &back, &layers)
                .unwrap();
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
                    if x < 2 {
                        assert_eq!(pixel, [255, 0, 0, 255]);
                    } else {
                        assert!(
                            (126..=128).contains(&pixel[0]) && (127..=129).contains(&pixel[1]),
                            "{pixel:?}"
                        );
                        assert_eq!(&pixel[2..], &[0, 255]);
                    }
                }
            }
            read.Unmap(0, None);
        }
    }
}
