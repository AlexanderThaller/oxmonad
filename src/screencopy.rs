//! Minimal `wlr-screencopy` support.
//!
//! This advertises SHM screencopy frames so tools such as `grim` can capture
//! compositor-drawn output. Frame data is rendered through the same software
//! path used by the snapshot tests.

use std::sync::atomic::{
    AtomicBool,
    Ordering,
};

use smithay::{
    output::Output,
    reexports::wayland_server::{
        Client,
        DataInit,
        Dispatch,
        DisplayHandle,
        GlobalDispatch,
        New,
        Resource,
        backend::GlobalId,
        protocol::{
            wl_buffer::WlBuffer,
            wl_output::WlOutput,
            wl_shm,
        },
    },
    utils::{
        Buffer,
        Logical,
        Rectangle,
    },
    wayland::shm::{
        BufferAccessError,
        BufferData,
        with_buffer_contents_mut,
    },
};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{
        self,
        ZwlrScreencopyFrameV1,
    },
    zwlr_screencopy_manager_v1::{
        self,
        ZwlrScreencopyManagerV1,
    },
};

use crate::{
    Oxmonad,
    snapshot,
};

const FORMAT: wl_shm::Format = wl_shm::Format::Argb8888;
const BYTES_PER_PIXEL: usize = 4;
const BYTES_PER_PIXEL_I32: i32 = 4;
const BYTES_PER_PIXEL_U32: u32 = 4;

#[derive(Debug)]
pub struct ScreencopyState {
    _global: GlobalId,
}

impl ScreencopyState {
    pub fn new<D>(display: &DisplayHandle) -> Self
    where
        D: GlobalDispatch<ZwlrScreencopyManagerV1, ()>
            + Dispatch<ZwlrScreencopyManagerV1, ()>
            + Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrame>
            + 'static,
    {
        Self {
            _global: display.create_global::<D, ZwlrScreencopyManagerV1, _>(3, ()),
        }
    }
}

#[derive(Debug)]
pub struct ScreencopyFrame {
    rect: Rectangle<i32, Buffer>,
    copied: AtomicBool,
}

#[derive(Debug, Clone)]
pub struct PendingScreencopy {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    rect: Rectangle<i32, Buffer>,
    report_damage: bool,
}

impl GlobalDispatch<ZwlrScreencopyManagerV1, ()> for Oxmonad {
    fn bind(
        _state: &mut Self,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for Oxmonad {
    fn request(
        _state: &mut Self,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor: _,
                output,
            } => init_frame(frame, &output, None, data_init),
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor: _,
                output,
                x,
                y,
                width,
                height,
            } => init_frame(
                frame,
                &output,
                Some(Rectangle::new((x, y).into(), (width, height).into())),
                data_init,
            ),
            zwlr_screencopy_manager_v1::Request::Destroy | _ => (),
        }
    }
}

impl Dispatch<ZwlrScreencopyFrameV1, ScreencopyFrame> for Oxmonad {
    fn request(
        state: &mut Self,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &ScreencopyFrame,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                copy_frame(state, frame, data, &buffer, false);
            }
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                copy_frame(state, frame, data, &buffer, true);
            }
            zwlr_screencopy_frame_v1::Request::Destroy | _ => (),
        }
    }
}

fn init_frame(
    frame: New<ZwlrScreencopyFrameV1>,
    output: &WlOutput,
    requested_region: Option<Rectangle<i32, Logical>>,
    data_init: &mut DataInit<'_, Oxmonad>,
) {
    let Some(output) = Output::from_resource(output) else {
        return;
    };

    let Some(mode) = output.current_mode() else {
        return;
    };

    let full = Rectangle::<i32, Logical>::from_size(mode.size.to_logical(1));
    let rect = requested_region.map_or(full, |region| {
        region
            .intersection(full)
            .unwrap_or_else(|| Rectangle::new(region.loc, (0, 0).into()))
    });
    let logical_size = full.size;
    let rect = rect.to_buffer(1, smithay::utils::Transform::Normal, &logical_size);
    let width = rect.size.w.max(0).cast_unsigned();
    let height = rect.size.h.max(0).cast_unsigned();
    let stride = width.saturating_mul(BYTES_PER_PIXEL_U32);
    let frame = data_init.init(
        frame,
        ScreencopyFrame {
            rect,
            copied: AtomicBool::new(false),
        },
    );

    frame.buffer(FORMAT, width, height, stride);
    if frame.version() >= 3 {
        frame.buffer_done();
    }
}

fn copy_frame(
    state: &mut Oxmonad,
    frame: &ZwlrScreencopyFrameV1,
    data: &ScreencopyFrame,
    buffer: &WlBuffer,
    report_damage: bool,
) {
    if data.copied.swap(true, Ordering::AcqRel) {
        frame.post_error(
            zwlr_screencopy_frame_v1::Error::AlreadyUsed,
            "screencopy frame already used",
        );
        return;
    }

    if data.rect.size.w <= 0 || data.rect.size.h <= 0 {
        frame.failed();
        return;
    }

    state.pending_screencopies.push(PendingScreencopy {
        frame: frame.clone(),
        buffer: buffer.clone(),
        rect: data.rect,
        report_damage,
    });
}

pub fn finish_pending_screencopies(
    state: &mut Oxmonad,
    rendered: Option<&crate::state::RenderedFrame>,
) {
    let pending = std::mem::take(&mut state.pending_screencopies);
    for copy in pending {
        let Some(rendered) = rendered else {
            copy.frame.failed();
            continue;
        };

        finish_pending_screencopy(state.start_time.elapsed(), copy, rendered);
    }
}

pub fn finish_pending_screencopies_for_output(state: &mut Oxmonad, output: &Output, scale: f64) {
    if state.pending_screencopies.is_empty() {
        return;
    }

    let Some(mode) = output.current_mode() else {
        finish_pending_screencopies(state, None);
        return;
    };

    let elements = state.overlay_elements(output, scale);

    let width = mode.size.w.max(0).cast_unsigned();
    let height = mode.size.h.max(0).cast_unsigned();
    let rendered = snapshot::render_to_frame_with_transform(
        width,
        height,
        output.current_transform(),
        &elements,
    );

    finish_pending_screencopies(state, Some(&rendered));
}

fn finish_pending_screencopy(
    elapsed: std::time::Duration,
    copy: PendingScreencopy,
    rendered: &crate::state::RenderedFrame,
) {
    let PendingScreencopy {
        frame,
        buffer,
        rect,
        report_damage,
    } = copy;

    match with_buffer_contents_mut(&buffer, |ptr, len, metadata| {
        copy_to_shm(ptr, len, metadata, rendered, rect)
    }) {
        Ok(Ok(())) => {
            if report_damage {
                frame.damage(
                    0,
                    0,
                    rect.size.w.cast_unsigned(),
                    rect.size.h.cast_unsigned(),
                );
            }
            frame.flags(zwlr_screencopy_frame_v1::Flags::empty());
            let seconds = elapsed.as_secs();
            frame.ready(
                u32::try_from(seconds >> 32).expect("high 32 bits fit in u32"),
                u32::try_from(seconds & u64::from(u32::MAX)).expect("low 32 bits fit in u32"),
                elapsed.subsec_nanos(),
            );
        }
        Ok(Err(())) => {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "screencopy buffer has incompatible attributes",
            );
        }
        Err(BufferAccessError::BadMap) => frame.failed(),
        Err(_) => {
            frame.post_error(
                zwlr_screencopy_frame_v1::Error::InvalidBuffer,
                "screencopy only supports wl_shm buffers",
            );
        }
    }
}

fn copy_to_shm(
    ptr: *mut u8,
    len: usize,
    metadata: BufferData,
    rendered: &crate::state::RenderedFrame,
    rect: Rectangle<i32, Buffer>,
) -> Result<(), ()> {
    if metadata.format != FORMAT
        || metadata.width != rect.size.w
        || metadata.height != rect.size.h
        || metadata.stride < rect.size.w * BYTES_PER_PIXEL_I32
    {
        return Err(());
    }

    let offset = usize::try_from(metadata.offset).map_err(|_| ())?;
    let stride = usize::try_from(metadata.stride).map_err(|_| ())?;
    let width = usize::try_from(rect.size.w).map_err(|_| ())?;
    let height = usize::try_from(rect.size.h).map_err(|_| ())?;
    let src_x = usize::try_from(rect.loc.x).map_err(|_| ())?;
    let src_y = usize::try_from(rect.loc.y).map_err(|_| ())?;
    let rendered_width = usize::try_from(rendered.size.w).map_err(|_| ())?;
    let rendered_height = usize::try_from(rendered.size.h).map_err(|_| ())?;
    let row_len = width.checked_mul(BYTES_PER_PIXEL).ok_or(())?;
    let dst_len = offset
        .checked_add(stride.checked_mul(height.saturating_sub(1)).ok_or(())?)
        .and_then(|end| end.checked_add(row_len))
        .ok_or(())?;
    if dst_len > len {
        return Err(());
    }

    if src_x.checked_add(width).ok_or(())? > rendered_width
        || src_y.checked_add(height).ok_or(())? > rendered_height
    {
        return Err(());
    }

    for row in 0..height {
        let src_start = (src_y + row)
            .checked_mul(rendered.stride)
            .and_then(|row_start| row_start.checked_add(src_x * BYTES_PER_PIXEL))
            .ok_or(())?;
        let dst_start = offset
            .checked_add(row.checked_mul(stride).ok_or(())?)
            .ok_or(())?;
        let src = &rendered.pixels[src_start..src_start + row_len];

        // SAFETY: Smithay's SHM helper guarantees `ptr..ptr+len` is mapped and
        // writable for the callback duration. Bounds above ensure this row is
        // inside that mapped range.
        #[expect(
            unsafe_code,
            reason = "Copying into the wl_shm pool requires writing through Smithay's callback \
                      pointer."
        )]
        unsafe {
            std::ptr::copy_nonoverlapping(src.as_ptr(), ptr.add(dst_start), row_len);
        }
    }

    Ok(())
}
