use std::slice;

use voltshot_core::{CapturedImage, CoreError};
use windows::{
    Win32::Graphics::{
        Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
        Direct3D11::{
            D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
            D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
            D3D11CreateDevice, ID3D11DeviceContext, ID3D11Texture2D,
        },
        Dxgi::{
            Common::{
                DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION, DXGI_MODE_ROTATION_IDENTITY,
                DXGI_MODE_ROTATION_ROTATE90, DXGI_MODE_ROTATION_ROTATE180,
                DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_UNSPECIFIED,
            },
            CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT,
            DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput,
            IDXGIOutput1, IDXGIOutputDuplication,
        },
    },
    core::{Error, Interface},
};

const FRAME_TIMEOUT_MS: u32 = 25;
const FRAME_ATTEMPTS: u32 = 3;

/// Captures the attached output at the given zero-based index in DXGI adapter/output order.
pub fn capture_monitor_dxgi(output_index: u32) -> Result<CapturedImage, CoreError> {
    capture_output(|index, _| index == output_index)
}

pub(crate) fn capture_named_monitor(name: &str) -> Result<CapturedImage, CoreError> {
    let name: Vec<_> = name.encode_utf16().collect();
    capture_output(|_, desc| {
        desc.DeviceName.split(|&unit| unit == 0).next() == Some(name.as_slice())
    })
}

fn find_output(
    mut matches: impl FnMut(u32, &DXGI_OUTPUT_DESC) -> bool,
) -> Result<(IDXGIAdapter1, IDXGIOutput, DXGI_OUTPUT_DESC), CoreError> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }.map_err(dxgi_error)?;
    let mut attached_index = 0;
    for adapter_index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => return Err(dxgi_error(error)),
        };
        for output_index in 0.. {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(error) => return Err(dxgi_error(error)),
            };
            let desc = unsafe { output.GetDesc() }.map_err(dxgi_error)?;
            if desc.AttachedToDesktop.as_bool() {
                if matches(attached_index, &desc) {
                    return Ok((adapter, output, desc));
                }
                attached_index += 1;
            }
        }
    }
    Err(CoreError::CaptureFailed(
        "DXGI desktop output not found".into(),
    ))
}

fn capture_output(
    matches: impl FnMut(u32, &DXGI_OUTPUT_DESC) -> bool,
) -> Result<CapturedImage, CoreError> {
    let (adapter, output, output_desc) = find_output(matches)?;
    let output: IDXGIOutput1 = output.cast().map_err(dxgi_error)?;
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            &adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(dxgi_error)?;
    let device = device.ok_or_else(|| missing("D3D11 device"))?;
    let context = context.ok_or_else(|| missing("D3D11 device context"))?;
    let duplication = unsafe { output.DuplicateOutput(&device) }.map_err(dxgi_error)?;

    let (_frame, source) = acquire_presented_frame(&duplication)?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { source.GetDesc(&mut desc) };
    if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
        return Err(CoreError::CaptureFailed(
            "unexpected DXGI desktop pixel format".into(),
        ));
    }
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
        ..desc
    };
    let mut staging = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
        .map_err(dxgi_error)?;
    let staging = staging.ok_or_else(|| missing("staging texture"))?;
    unsafe { context.CopyResource(&staging, &source) };

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(dxgi_error)?;
    let _mapping = MappedTexture(&context, &staging);
    let pitch = mapped.RowPitch as usize;
    let row_bytes = (desc.Width as usize).checked_mul(4);
    let row_bytes = row_bytes.filter(|&bytes| bytes <= pitch && bytes != 0);
    let height = desc.Height as usize;
    let valid_span = height != 0
        && height
            .checked_mul(pitch)
            .is_some_and(|span| span <= isize::MAX as usize);
    let row_bytes = row_bytes
        .filter(|_| !mapped.pData.is_null() && valid_span)
        .ok_or_else(|| CoreError::CaptureFailed("invalid DXGI staging texture layout".into()))?;
    let base = mapped.pData.cast::<u8>();
    let rows =
        (0..height).map(|row| unsafe { slice::from_raw_parts(base.add(row * pitch), row_bytes) });
    convert_bgra(desc.Width, desc.Height, output_desc.Rotation, rows)
}

fn convert_bgra<'a>(
    width: u32,
    height: u32,
    rotation: DXGI_MODE_ROTATION,
    rows: impl Iterator<Item = &'a [u8]>,
) -> Result<CapturedImage, CoreError> {
    let (out_width, out_height) = match rotation {
        DXGI_MODE_ROTATION_ROTATE90 | DXGI_MODE_ROTATION_ROTATE270 => (height, width),
        DXGI_MODE_ROTATION_UNSPECIFIED
        | DXGI_MODE_ROTATION_IDENTITY
        | DXGI_MODE_ROTATION_ROTATE180 => (width, height),
        _ => {
            return Err(CoreError::CaptureFailed(
                "unknown DXGI output rotation".into(),
            ));
        }
    };
    let len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| CoreError::CaptureFailed("DXGI frame is too large".into()))?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(len).map_err(|error| {
        CoreError::CaptureFailed(format!("cannot allocate DXGI frame: {error}"))
    })?;
    rgba.resize(len, 0);
    let source_width = width as usize;
    let source_height = height as usize;
    let dest_width = out_width as usize;
    for (row, bytes) in rows.enumerate() {
        for (column, pixel) in bytes.chunks_exact(4).enumerate() {
            let (dest_column, dest_row) = match rotation {
                DXGI_MODE_ROTATION_ROTATE90 => (source_height - 1 - row, column),
                DXGI_MODE_ROTATION_ROTATE180 => {
                    (source_width - 1 - column, source_height - 1 - row)
                }
                DXGI_MODE_ROTATION_ROTATE270 => (row, source_width - 1 - column),
                _ => (column, row),
            };
            let offset = (dest_row * dest_width + dest_column) * 4;
            rgba[offset..offset + 4].copy_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
    }
    CapturedImage::from_rgba(out_width, out_height, rgba)
}

fn dxgi_error(error: Error) -> CoreError {
    CoreError::CaptureFailed(format!("DXGI desktop capture: {error}"))
}

fn missing(resource: &str) -> CoreError {
    CoreError::CaptureFailed(format!("DXGI did not return a {resource}"))
}

fn acquire_presented_frame(
    duplication: &IDXGIOutputDuplication,
) -> Result<(AcquiredFrame<'_>, ID3D11Texture2D), CoreError> {
    let mut saw_unpresented_frame = false;
    for _ in 0..FRAME_ATTEMPTS {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        match unsafe {
            duplication.AcquireNextFrame(FRAME_TIMEOUT_MS, &mut frame_info, &mut resource)
        } {
            Ok(()) => {}
            Err(error) if error.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(error) => return Err(dxgi_error(error)),
        }
        let frame = AcquiredFrame(duplication);
        if frame_info.LastPresentTime == 0 {
            saw_unpresented_frame = true;
            continue;
        }
        let source = resource
            .ok_or_else(|| missing("desktop frame"))?
            .cast()
            .map_err(dxgi_error)?;
        return Ok((frame, source));
    }
    if saw_unpresented_frame {
        return Err(CoreError::CaptureFailed(
            "DXGI returned unpresented frame".into(),
        ));
    }
    Err(dxgi_error(Error::from_hresult(DXGI_ERROR_WAIT_TIMEOUT)))
}

struct AcquiredFrame<'a>(&'a IDXGIOutputDuplication);

impl Drop for AcquiredFrame<'_> {
    fn drop(&mut self) {
        let _ = unsafe { self.0.ReleaseFrame() };
    }
}

struct MappedTexture<'a>(&'a ID3D11DeviceContext, &'a ID3D11Texture2D);

impl Drop for MappedTexture<'_> {
    fn drop(&mut self) {
        unsafe { self.0.Unmap(self.1, 0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_pitched_bgra_and_rotates_the_output() {
        let pitched = [
            10, 20, 1, 255, 11, 21, 2, 254, 99, 99, 99, 99, 12, 22, 3, 253, 13, 23, 4, 252, 99, 99,
            99, 99, 14, 24, 5, 251, 15, 25, 6, 250, 99, 99, 99, 99,
        ];
        let pixels = [
            [1, 20, 10, 255],
            [2, 21, 11, 254],
            [3, 22, 12, 253],
            [4, 23, 13, 252],
            [5, 24, 14, 251],
            [6, 25, 15, 250],
        ];
        for (rotation, size, order) in [
            (DXGI_MODE_ROTATION_IDENTITY, (2, 3), [0, 1, 2, 3, 4, 5]),
            (DXGI_MODE_ROTATION_ROTATE90, (3, 2), [4, 2, 0, 5, 3, 1]),
            (DXGI_MODE_ROTATION_ROTATE180, (2, 3), [5, 4, 3, 2, 1, 0]),
            (DXGI_MODE_ROTATION_ROTATE270, (3, 2), [1, 3, 5, 0, 2, 4]),
        ] {
            let rows = pitched.chunks_exact(12).map(|row| &row[..8]);
            let image = convert_bgra(2, 3, rotation, rows).expect("valid texture");
            assert_eq!((image.width(), image.height()), size);
            assert_eq!(image.rgba(), order.map(|index| pixels[index]).concat());
        }
    }
}
