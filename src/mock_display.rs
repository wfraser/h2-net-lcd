use anyhow::Result;

pub fn init_display() -> Result<MockDisplay> {
    Ok(MockDisplay::new())
}

pub fn stop_display(_: MockDisplay) {}

pub struct MockDisplay {
    lines: Vec<Vec<char>>,
    pos: (usize, usize),
}

impl MockDisplay {
    pub fn new() -> Self {
        Self {
            lines: vec![vec![' '; 20]; 4],
            pos: (0, 0),
        }
    }

    pub fn position(&mut self, col: u8, row: u8) {
        self.pos = (row.min(3) as usize, col.min(19) as usize);
    }

    pub fn print(&mut self, s: &str) {
        for &byte in s.as_bytes() {
            self.write(byte);
        }
    }

    pub fn write(&mut self, byte: u8) {
        let c = match byte {
            0 => ' ',
            1 ..= 7 => std::char::from_u32(0x2580 + byte as u32).unwrap(),
            0xdf => '°',
            _ => byte as char,
        };

        self.lines[self.pos.0][self.pos.1] = c;
        self.pos.1 += 1;
        if self.pos.1 == 20 {
            self.pos.0 += 1;
            self.pos.1 = 0;
        }
        if self.pos.0 == 4 {
            self.pos.0 = 0;
        }
    }

    pub fn dump(&self) {
        for line in &self.lines {
            for c in line.iter() {
                print!("{}", c);
            }
            println!();
        }
    }
}

impl std::fmt::Write for MockDisplay {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.print(s);
        Ok(())
    }
}
