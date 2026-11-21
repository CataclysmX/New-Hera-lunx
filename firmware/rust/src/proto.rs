//! Streaming parser of the display command stream (docs/firmware/protocol.md).
//!
//! Byte-oriented state machine: messages may span USB packets, nothing is buffered except
//! the 8-byte header, the rectangle and fixed payloads. A bad sync byte is skipped.

pub const SYNC: u8 = 0xA5;

pub mod op {
    pub const FILL: u8 = 0x01;
    pub const BLIT: u8 = 0x02;
    pub const BLIT_RLE: u8 = 0x03;
    /// Copy a rectangle of the framebuffer elsewhere (scrolling), 240.21.
    pub const COPY: u8 = 0x04;
    pub const BACKLIGHT: u8 = 0x10;
    pub const LEDS: u8 = 0x11;
}

/// What the parser drives.
pub trait Sink {
    fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, color: u16);
    fn pixel(&mut self, x: usize, y: usize, color: u16);
    /// `px.len() / 2` little-endian RGB565 pixels on row `y` from column `x` (fast path).
    fn span(&mut self, x: usize, y: usize, px: &[u8]) {
        for (i, p) in px.chunks_exact(2).enumerate() {
            self.pixel(x + i, y, u16::from_le_bytes([p[0], p[1]]));
        }
    }
    /// `n` pixels of `color` on row `y` from column `x` (fast path).
    fn hline(&mut self, x: usize, y: usize, n: usize, color: u16) {
        for i in 0..n {
            self.pixel(x + i, y, color);
        }
    }
    /// Copy the rectangle (x, y, w, h) of the framebuffer so that its top-left corner goes to
    /// (to_x, to_y); source and destination may overlap.
    fn copy(&mut self, x: u16, y: u16, w: u16, h: u16, to_x: u16, to_y: u16);
    fn backlight(&mut self, pct: u8);
    fn leds(&mut self, pct: [u8; 4]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Header,
    Rect,
    Raw,
    Rle,
    Payload,
    Skip,
}

pub struct Parser {
    state: State,
    hdr: [u8; 8],
    n: usize,
    op: u8,
    remaining: u32,
    buf: [u8; 12],
    bn: usize,
    x0: u16,
    y0: u16,
    w: u16,
    h: u16,
    cx: u16,
    cy: u16,
    tmp: [u8; 3],
    tn: usize,
}

fn u16le(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

impl Parser {
    pub const fn new() -> Self {
        Parser {
            state: State::Header,
            hdr: [0; 8],
            n: 0,
            op: 0,
            remaining: 0,
            buf: [0; 12],
            bn: 0,
            x0: 0,
            y0: 0,
            w: 0,
            h: 0,
            cx: 0,
            cy: 0,
            tmp: [0; 3],
            tn: 0,
        }
    }

    pub fn feed(&mut self, data: &[u8], sink: &mut impl Sink) {
        let mut i = 0;
        while i < data.len() {
            let left = (data.len() - i).min(self.remaining as usize);
            match self.state {
                // Fast path: whole pixels up to the end of the current row.
                State::Raw if self.tn == 0 && self.cy < self.h && self.w > 0 && left >= 2 => {
                    let n = (left / 2).min((self.w - self.cx) as usize);
                    let x = self.x0 as usize + self.cx as usize;
                    sink.span(x, self.y0 as usize + self.cy as usize, &data[i..i + 2 * n]);
                    self.advance(n as u16);
                    self.consume(2 * n);
                    i += 2 * n;
                }
                State::Skip => {
                    self.consume(left);
                    i += left;
                }
                _ => {
                    self.byte(data[i], sink);
                    i += 1;
                }
            }
        }
    }

    fn consume(&mut self, n: usize) {
        self.remaining -= n as u32;
        if self.remaining == 0 {
            self.state = State::Header;
        }
    }

    /// Move the write cursor `n` pixels forward (n <= pixels left on the row).
    fn advance(&mut self, n: u16) {
        self.cx += n;
        if self.cx == self.w {
            self.cx = 0;
            self.cy += 1;
        }
    }

    /// `count` pixels of one colour, row segment by row segment.
    fn run(&mut self, color: u16, mut count: u16, sink: &mut impl Sink) {
        while count > 0 && self.cy < self.h && self.w > 0 {
            let n = count.min(self.w - self.cx);
            sink.hline(self.x0 as usize + self.cx as usize, self.y0 as usize + self.cy as usize, n as usize, color);
            self.advance(n);
            count -= n;
        }
    }

    fn byte(&mut self, b: u8, sink: &mut impl Sink) {
        if self.state == State::Header {
            if self.n == 0 && b != SYNC {
                return; // resync
            }
            self.hdr[self.n] = b;
            self.n += 1;
            if self.n == 8 {
                self.n = 0;
                self.op = self.hdr[1];
                self.remaining = u32::from_le_bytes([self.hdr[4], self.hdr[5], self.hdr[6], self.hdr[7]]);
                self.bn = 0;
                self.tn = 0;
                self.state = match self.op {
                    _ if self.remaining == 0 => State::Header,
                    op::FILL | op::COPY | op::BACKLIGHT | op::LEDS => State::Payload,
                    op::BLIT | op::BLIT_RLE => State::Rect,
                    _ => State::Skip,
                };
            }
            return;
        }

        self.remaining -= 1;
        match self.state {
            State::Rect => {
                self.buf[self.bn] = b;
                self.bn += 1;
                if self.bn == 8 {
                    self.x0 = u16le(&self.buf, 0);
                    self.y0 = u16le(&self.buf, 2);
                    self.w = u16le(&self.buf, 4);
                    self.h = u16le(&self.buf, 6);
                    self.cx = 0;
                    self.cy = 0;
                    self.state = if self.op == op::BLIT { State::Raw } else { State::Rle };
                }
            }
            State::Raw => {
                self.tmp[self.tn] = b;
                self.tn += 1;
                if self.tn == 2 {
                    self.tn = 0;
                    self.emit(u16::from_le_bytes([self.tmp[0], self.tmp[1]]), sink);
                }
            }
            State::Rle => {
                self.tmp[self.tn] = b;
                self.tn += 1;
                if self.tn == 3 {
                    self.tn = 0;
                    let color = u16::from_le_bytes([self.tmp[1], self.tmp[2]]);
                    self.run(color, self.tmp[0] as u16, sink);
                }
            }
            State::Payload => {
                if self.bn < self.buf.len() {
                    self.buf[self.bn] = b;
                    self.bn += 1;
                }
                if self.remaining == 0 {
                    self.execute(sink);
                }
            }
            State::Skip | State::Header => {}
        }
        if self.remaining == 0 {
            self.state = State::Header;
        }
    }

    fn emit(&mut self, color: u16, sink: &mut impl Sink) {
        if self.cy < self.h && self.w > 0 {
            sink.pixel(self.x0 as usize + self.cx as usize, self.y0 as usize + self.cy as usize, color);
            self.advance(1);
        }
    }

    fn execute(&mut self, sink: &mut impl Sink) {
        let b = &self.buf[..self.bn];
        match self.op {
            op::FILL if b.len() >= 10 => {
                sink.fill(u16le(b, 0), u16le(b, 2), u16le(b, 4), u16le(b, 6), u16le(b, 8))
            }
            op::COPY if b.len() >= 12 => sink.copy(
                u16le(b, 0),
                u16le(b, 2),
                u16le(b, 4),
                u16le(b, 6),
                u16le(b, 8),
                u16le(b, 10),
            ),
            op::BACKLIGHT if !b.is_empty() => sink.backlight(b[0]),
            op::LEDS if b.len() >= 4 => sink.leds([b[0], b[1], b[2], b[3]]),
            _ => {}
        }
    }
}
