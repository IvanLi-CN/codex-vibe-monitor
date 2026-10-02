use super::*;

pub(crate) struct PrefixedIo {
    prefix: std::io::Cursor<Vec<u8>>,
    inner: BoxedWsIo,
}

impl PrefixedIo {
    pub(super) fn new(prefix: Vec<u8>, inner: BoxedWsIo) -> Self {
        Self {
            prefix: std::io::Cursor::new(prefix),
            inner,
        }
    }
}

impl AsyncRead for PrefixedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let remaining = self.prefix.get_ref().len() as u64 - self.prefix.position();
        if remaining > 0 {
            let available = self.prefix.get_ref().len() - self.prefix.position() as usize;
            let to_copy = available.min(buf.remaining());
            let start = self.prefix.position() as usize;
            let end = start + to_copy;
            buf.put_slice(&self.prefix.get_ref()[start..end]);
            self.prefix.set_position(end as u64);
            return std::task::Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for PrefixedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
