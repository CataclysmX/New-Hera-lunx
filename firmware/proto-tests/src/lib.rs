//! Host tests of the firmware parser: `cd firmware/proto-tests && cargo test`.
//! The fast paths (row spans, RLE row segments, bulk skip) fed in random chunks must give the
//! same framebuffer as the plain byte-by-byte path on random command streams.

#[path = "../../rust/src/proto.rs"]
#[allow(dead_code)]
mod proto;

#[cfg(test)]
mod tests {
    use super::proto::{Parser, Sink};

    const W: usize = 480;
    const H: usize = 272;

    struct Fb {
        px: Vec<u16>,
        bl: u8,
        leds: [u8; 4],
        fast: bool,
    }

    impl Fb {
        fn new(fast: bool) -> Self {
            Fb { px: vec![0; W * H], bl: 0, leds: [0; 4], fast }
        }
    }

    impl Sink for Fb {
        fn fill(&mut self, x: u16, y: u16, w: u16, h: u16, c: u16) {
            for yy in y as usize..(y as usize + h as usize).min(H) {
                for xx in x as usize..(x as usize + w as usize).min(W) {
                    self.px[yy * W + xx] = c;
                }
            }
        }
        fn pixel(&mut self, x: usize, y: usize, c: u16) {
            if x < W && y < H {
                self.px[y * W + x] = c;
            }
        }
        fn span(&mut self, x: usize, y: usize, px: &[u8]) {
            if !self.fast {
                unreachable!("byte-by-byte feeding never takes the span path");
            }
            if x < W && y < H {
                for (i, p) in px.chunks_exact(2).take(W - x).enumerate() {
                    self.px[y * W + x + i] = u16::from_le_bytes([p[0], p[1]]);
                }
            }
        }
        fn hline(&mut self, x: usize, y: usize, n: usize, c: u16) {
            if x < W && y < H {
                for i in 0..n.min(W - x) {
                    self.px[y * W + x + i] = c;
                }
            }
        }
        // the pixels whose source and destination are both on the screen, as if read first
        // (overlapping rectangles)
        fn copy(&mut self, x: u16, y: u16, w: u16, h: u16, to_x: u16, to_y: u16) {
            let before = self.px.clone();
            for dy in 0..h as usize {
                for dx in 0..w as usize {
                    let (sx, sy) = (x as usize + dx, y as usize + dy);
                    let (tx, ty) = (to_x as usize + dx, to_y as usize + dy);
                    if sx < W && sy < H && tx < W && ty < H {
                        self.px[ty * W + tx] = before[sy * W + sx];
                    }
                }
            }
        }
        fn backlight(&mut self, pct: u8) {
            self.bl = pct;
        }
        fn leds(&mut self, pct: [u8; 4]) {
            self.leds = pct;
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
        fn below(&mut self, n: u32) -> u32 {
            self.next() % n
        }
    }

    fn msg(op: u8, payload: &[u8], out: &mut Vec<u8>) {
        out.extend_from_slice(&[0xA5, op, 0, 0]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
    }

    fn rect(r: &mut Rng) -> Vec<u8> {
        // mostly on screen, sometimes clipped or degenerate
        let x = if r.below(8) == 0 { 400 + r.below(200) } else { r.below(480) } as u16;
        let y = if r.below(8) == 0 { 200 + r.below(200) } else { r.below(272) } as u16;
        let w = if r.below(16) == 0 { 0 } else { 1 + r.below(120) } as u16;
        let h = if r.below(16) == 0 { 0 } else { 1 + r.below(60) } as u16;
        [x, y, w, h].iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn stream(r: &mut Rng) -> Vec<u8> {
        let mut s = Vec::new();
        for _ in 0..200 {
            match r.below(8) {
                0 => {
                    let mut p = rect(r);
                    p.extend_from_slice(&(r.next() as u16).to_le_bytes());
                    msg(0x01, &p, &mut s);
                }
                1 | 2 => {
                    let mut p = rect(r);
                    let (w, h) = (u16::from_le_bytes([p[4], p[5]]), u16::from_le_bytes([p[6], p[7]]));
                    // exact, short or overlong pixel payloads (odd lengths included)
                    let n = (w as usize * h as usize * 2) as i64 + [0i64, 0, -3, 5][r.below(4) as usize];
                    for _ in 0..n.max(0) {
                        p.push(r.next() as u8);
                    }
                    msg(0x02, &p, &mut s);
                }
                3 | 4 => {
                    let mut p = rect(r);
                    for _ in 0..r.below(300) {
                        p.push(r.below(256) as u8);
                        p.extend_from_slice(&(r.next() as u16).to_le_bytes());
                    }
                    msg(0x03, &p, &mut s);
                }
                5 if r.below(2) == 0 => {
                    // COPY: a rectangle and where it goes
                    let mut p = rect(r);
                    p.extend_from_slice(&(r.below(480) as u16).to_le_bytes());
                    p.extend_from_slice(&(r.below(272) as u16).to_le_bytes());
                    msg(0x04, &p, &mut s);
                }
                5 => msg(0x10, &[r.below(120) as u8], &mut s),
                6 => msg(0x11, &[1, 2, 3, r.below(100) as u8], &mut s),
                _ => {
                    if r.below(2) == 0 {
                        let junk: Vec<u8> = (0..r.below(700)).map(|_| r.next() as u8).collect();
                        msg(0x7F, &junk, &mut s); // NOP / unknown op: skipped
                    } else {
                        s.extend((0..r.below(5)).map(|_| 0x5A)); // garbage between messages
                    }
                }
            }
        }
        s
    }

    #[test]
    fn fast_paths_match_byte_by_byte() {
        let mut r = Rng(0x1234_5678_9abc_def1);
        for round in 0..40 {
            let s = stream(&mut r);
            let mut slow = Fb::new(false);
            let mut p = Parser::new();
            for b in &s {
                p.feed(core::slice::from_ref(b), &mut slow);
            }
            let mut fast = Fb::new(true);
            let mut p = Parser::new();
            let mut i = 0;
            while i < s.len() {
                let n = [1, 2, 3, 63, 64, 512, 4096][r.below(7) as usize].min(s.len() - i);
                p.feed(&s[i..i + n], &mut fast);
                i += n;
            }
            assert!(slow.px == fast.px, "framebuffer mismatch in round {round}");
            assert_eq!((slow.bl, slow.leds), (fast.bl, fast.leds), "round {round}");
        }
    }

    #[test]
    fn full_screen_blit() {
        let mut p = Vec::new();
        p.extend([0u16, 0, 480, 272].iter().flat_map(|v| v.to_le_bytes()));
        for i in 0..W * H {
            p.extend_from_slice(&(i as u16).to_le_bytes());
        }
        let mut s = Vec::new();
        msg(0x02, &p, &mut s);
        let mut fb = Fb::new(true);
        let mut parser = Parser::new();
        for c in s.chunks(512) {
            parser.feed(c, &mut fb);
        }
        assert!(fb.px.iter().enumerate().all(|(i, &v)| v == i as u16));
    }
}
