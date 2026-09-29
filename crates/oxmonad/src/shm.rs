//! `wl_shm` buffers for the WM-drawn surfaces.
//!
//! Each buffer lives in its own memfd; pixels are copied in with `pwrite`
//! rather than through a shared mapping, which keeps this free of `unsafe`
//! and is cheap for bar-sized surfaces. A buffer is reused once the
//! compositor released it.

use std::{
    fs::File,
    io,
    os::{
        fd::AsFd,
        unix::fs::FileExt,
    },
    sync::{
        Arc,
        atomic::{
            AtomicBool,
            Ordering,
        },
    },
};

use oxmonad_ui::Canvas;
use rustix::fs::{
    MemfdFlags,
    memfd_create,
};
use wayland_client::{
    Dispatch,
    QueueHandle,
    protocol::{
        wl_buffer::WlBuffer,
        wl_shm::{
            Format,
            WlShm,
        },
        wl_shm_pool::WlShmPool,
    },
};

/// Set while the compositor holds the buffer; cleared on `wl_buffer.release`.
pub type BusyFlag = Arc<AtomicBool>;

#[derive(Debug)]
struct Buffer {
    proxy: WlBuffer,
    pool: WlShmPool,
    file: File,
    busy: BusyFlag,
    width: u32,
    height: u32,
}

impl Buffer {
    fn destroy(self) {
        self.proxy.destroy();
        self.pool.destroy();
    }
}

/// The buffers of one surface.
#[derive(Debug, Default)]
pub struct Buffers(Vec<Buffer>);

/// At most this many buffers are kept per surface.
const MAX_BUFFERS: usize = 3;

impl Buffers {
    /// A buffer holding `canvas`, marked busy; attach it and commit.
    pub fn upload<D>(
        &mut self,
        shm: &WlShm,
        canvas: &Canvas,
        qh: &QueueHandle<D>,
    ) -> io::Result<WlBuffer>
    where
        D: Dispatch<WlShmPool, ()> + Dispatch<WlBuffer, BusyFlag> + 'static,
    {
        let (width, height) = (canvas.width(), canvas.height());
        // Drop idle buffers of another size.
        let (keep, stale): (Vec<Buffer>, Vec<Buffer>) =
            std::mem::take(&mut self.0).into_iter().partition(|b| {
                b.busy.load(Ordering::Acquire) || (b.width, b.height) == (width, height)
            });
        self.0 = keep;
        stale.into_iter().for_each(Buffer::destroy);

        let idle = self.0.iter().position(|b| {
            !b.busy.load(Ordering::Acquire) && (b.width, b.height) == (width, height)
        });
        let index = if let Some(index) = idle {
            index
        } else {
            if self.0.len() >= MAX_BUFFERS {
                // All busy: the compositor is slow; replace the oldest.
                self.0.remove(0).destroy();
            }
            self.0.push(create(shm, canvas, qh)?);
            self.0.len() - 1
        };
        let buffer = &self.0[index];
        buffer.file.write_all_at(canvas.as_bytes(), 0)?;
        buffer.busy.store(true, Ordering::Release);
        Ok(buffer.proxy.clone())
    }

    /// Destroys every buffer.
    pub fn destroy(&mut self) {
        self.0.drain(..).for_each(Buffer::destroy);
    }
}

fn create<D>(shm: &WlShm, canvas: &Canvas, qh: &QueueHandle<D>) -> io::Result<Buffer>
where
    D: Dispatch<WlShmPool, ()> + Dispatch<WlBuffer, BusyFlag> + 'static,
{
    let (width, height, stride) = (canvas.width(), canvas.height(), canvas.stride());
    let size = u64::from(stride) * u64::from(height);
    let too_large = || io::Error::new(io::ErrorKind::InvalidInput, "buffer too large");
    let size_i32 = i32::try_from(size).map_err(|_| too_large())?;
    let fd = memfd_create(
        "oxmonad-shm",
        MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING,
    )?;
    let file = File::from(fd);
    file.set_len(size)?;
    let pool = shm.create_pool(file.as_fd(), size_i32, qh, ());
    let busy = BusyFlag::default();
    let buffer = pool.create_buffer(
        0,
        i32::try_from(width).map_err(|_| too_large())?,
        i32::try_from(height).map_err(|_| too_large())?,
        i32::try_from(stride).map_err(|_| too_large())?,
        Format::Argb8888,
        qh,
        busy.clone(),
    );
    Ok(Buffer {
        proxy: buffer,
        pool,
        file,
        busy,
        width,
        height,
    })
}
