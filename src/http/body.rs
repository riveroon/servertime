use std::{pin::Pin, task::{Context, Poll}};
use futures_lite::ready;
use tokio::io::{self, AsyncRead, AsyncBufRead, ErrorKind, ReadBuf};

#[derive(Debug)]
struct Chunked<'a, S> {
    stream: Pin<&'a mut S>,
    buf: &'a mut Vec<u8>,
    pos: usize,
    remaining_size: usize
}

impl<'a, S> Chunked<'a, S> {
    fn new(stream: Pin<&'a mut S>, buf: &'a mut Vec<u8>, pos: usize) -> Self {
        let mut this = Self {
            stream, buf,
            pos: 0,
            remaining_size: 0
        };

        Pin::new(&mut this).consume_buf(pos - 2);
        this
    }

    /// Returns whether the parse was successful
    fn parse_buf(&mut self) -> io::Result<bool> {
        use httparse::{Status, parse_chunk_size};

        //httparse rejects single LFs anyways
        let pos_next = self.pos + 2;

        let (pos, len) = match parse_chunk_size(&self.buf[pos_next..]) {
            Ok(Status::Complete((_, 0))) => return Ok(true),
            Ok(Status::Complete(x)) => x,
            Ok(Status::Partial) => return Ok(false),
            Err(_) => Err(io::Error::new(ErrorKind::InvalidData, "Invalid Chunk Size"))?
        };

        self.pos = pos_next + pos;
        self.remaining_size += len as usize;

        Ok(true)
    }

    ///returns whether the next chunk is available
    fn prepare_next_chunk(&mut self) -> io::Result<bool> {
        //Ok( [self.pos < self.buf.len(), self.pos + 2 < self.buf.len() && self.parse_buf()?][(self.remaining_size == 0) as usize] )

        let ready = match self.remaining_size {
            0 => self.pos + 2 < self.buf.len() && self.parse_buf()?,
            _ => self.pos < self.buf.len()
        };

        Ok(ready)
    }

    fn next_chunk(&mut self) -> io::Result<&[u8]> {
        let len = (self.remaining_size).min(self.buf.len() - self.pos);

        Ok( &self.buf[self.pos..self.pos+len] )
    }

    fn consume_buf(&mut self, count: usize) {
        //Move all remaining bytes to the front; Same as this.buf.drain(..*count)
        unsafe {
            let dst = self.buf.as_mut_ptr();
            let src = dst.add(count);

            let len = self.buf.len() - count;

            //SAFETY: both src and dst are initialized inside the vector.
            std::ptr::copy(src, dst, len);

            //SAFETY: new_len is less then the old len (count - pos),
            //and is initialized (ptr::copy)
            self.buf.set_len(len);
        }

        self.pos = 0;
    }
}

impl<S: AsyncRead> Chunked<'_, S> {
    fn fill_buf(&mut self, cx: &mut Context) -> Poll<io::Result<()>> {
        self.consume_buf(self.pos);

        let mut buf = ReadBuf::uninit(self.buf.spare_capacity_mut());

        match self.stream.as_mut().poll_read(cx, &mut buf) {
            Poll::Ready(Ok(())) => (),
            x => return x
        }

        let len = buf.filled().len();

        unsafe { self.buf.set_len(self.buf.len() + len) };

        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead> AsyncRead for Chunked<'_, S> {
    fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let rem = buf.remaining();

            let chunk = match ready!(self.as_mut().poll_fill_buf(cx))? {
                &[] => break,
                x => x
            };
            let len = chunk.len();

            if rem > len {
                buf.put_slice(chunk);
                self.as_mut().consume(len);
            } else {
                buf.put_slice(&chunk[..rem]);
                self.as_mut().consume(rem);

                break;
            }
        }

        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead> AsyncBufRead for Chunked<'_, S> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<&[u8]>> {
        let this = self.get_mut();

        if !this.prepare_next_chunk()? {
            ready!(this.fill_buf(cx))?;
            this.prepare_next_chunk()?;
        }

        Poll::Ready(this.next_chunk())
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        self.pos += amt;
        self.remaining_size -= amt;
    }
}


#[derive(Debug)]
struct Stream<'a, S> {
    stream: Pin<&'a mut S>,
    buf: &'a mut Vec<u8>,
    pos: usize,
    len: usize
}

impl<'a, S> Stream<'a, S> {
    fn new(stream: Pin<&'a mut S>, buf: &'a mut Vec<u8>, pos: usize, len: usize) -> Self {
        Self {
            stream, buf, pos, len
        }
    }

    fn clear_buf(&mut self) {
        self.buf.clear();
        self.pos = 0;
    }
}

impl<S: AsyncRead> AsyncRead for Stream<'_, S> {
    fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
        // If we don't have any buffered data and we're doing a massive read
        // (larger than our internal buffer), bypass our internal buffer
        // entirely.
        if self.pos == self.buf.len() &&
            buf.remaining() >= self.buf.spare_capacity_mut().len() &&
            self.len > 0
        {
            let prev_len = buf.filled().len();

            let stream = self.stream.as_mut();
            let res = ready!(stream.poll_read(cx, buf));

            self.len -= buf.filled().len() - prev_len;
            self.clear_buf();

            return Poll::Ready(res);
        }

        let rem = ready!(self.as_mut().poll_fill_buf(cx))?;
        let amt = rem.len().min(buf.remaining());
        buf.put_slice(&rem[..amt]);
        self.consume(amt);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncRead> AsyncBufRead for Stream<'_, S> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();

        if this.pos >= this.buf.len() && this.len > 0 {
            this.clear_buf();

            let mut buf = {
                let uninit = this.buf.spare_capacity_mut();
                let len = uninit.len().min(this.len);
                ReadBuf::uninit(&mut uninit[..len])
            };

            ready!(this.stream.as_mut().poll_read(cx, &mut buf))?;

            let len = buf.filled().len();

            unsafe { this.buf.set_len(len) }
        }

        Poll::Ready(Ok(&this.buf[this.pos..]))
    }

    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        self.pos += amt;
        self.len -= amt;
    }
}


enum BodyInner<'a, S> {
    Stream(Stream<'a, S>),
    Chunked(Chunked<'a, S>)
}

pub struct Body<'a, S> {
    inner: BodyInner<'a, S>
}

impl<'a, S> Body<'a, S> {
    pub(crate) fn new(
        stream: Pin<&'a mut S>,
        buf: &'a mut Vec<u8>,
        pos: usize,
        len: Option<usize>) -> Self {
        
        let inner = match len {
            Some(len) => BodyInner::Stream(Stream::new(stream, buf, pos, len)),
            None => BodyInner::Chunked(Chunked::new(stream, buf, pos))
        };

        Self { inner }
    }
}

impl<S: AsyncRead> Body<'_, S> {
    pub async fn discard(mut self) -> io::Result<u64> {
        io::copy(&mut self, &mut io::sink()).await
    }
}

impl<S: AsyncRead> AsyncRead for Body<'_, S> {
    fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
        match &mut self.inner {
            BodyInner::Stream(s) => Pin::new(s).poll_read(cx, buf),
            BodyInner::Chunked(s) => Pin::new(s).poll_read(cx, buf)
        }
    }
}

impl<S: AsyncRead> AsyncBufRead for Body<'_, S> {
    fn poll_fill_buf(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();
        match &mut this.inner {
            BodyInner::Stream(s) => Pin::new(s).poll_fill_buf(cx),
            BodyInner::Chunked(s) => Pin::new(s).poll_fill_buf(cx)
        }
    }
    
    fn consume(mut self: Pin<&mut Self>, amt: usize) {
        match &mut self.inner {
            BodyInner::Stream(s) => Pin::new(s).consume(amt),
            BodyInner::Chunked(s) => Pin::new(s).consume(amt)
        }
    }
}