pub fn draw_char(
    buffer: &mut [u32],
    fb_width: usize,
    x: usize,
    y: usize,
    c: char,
    color: u32,
) {
    let rows = match glyph_rows(c) {
        Some(rows) => rows,
        None => return,
    };

    let fb_height = if fb_width == 0 { 0 } else { buffer.len() / fb_width };

    for (row_idx, bits) in rows.iter().enumerate() {
        let py = y + row_idx;
        if py >= fb_height {
            continue;
        }

        for col in 0..5 {
            if (bits >> (4 - col)) & 1 == 1 {
                let px = x + col;
                if px < fb_width {
                    buffer[py * fb_width + px] = color;
                }
            }
        }
    }
}

pub fn draw_text(
    buffer: &mut [u32],
    fb_width: usize,
    x: usize,
    y: usize,
    text: &str,
    color: u32,
) {
    let mut cx = x;
    for ch in text.chars() {
        draw_char(buffer, fb_width, cx, y, ch, color);
        cx += 6; // 5 px glyph + 1 px spacing
    }
}

pub fn draw_char_2x(
    buffer: &mut [u32],
    fb_width: usize,
    x: usize,
    y: usize,
    c: char,
    color: u32,
) {
    let rows = match glyph_rows(c) {
        Some(rows) => rows,
        None => return,
    };

    let fb_height = if fb_width == 0 { 0 } else { buffer.len() / fb_width };

    for (row_idx, bits) in rows.iter().enumerate() {
        for col in 0..5 {
            if (bits >> (4 - col)) & 1 == 1 {
                let px = x + col * 2;
                let py = y + row_idx * 2;

                for dy in 0..2 {
                    for dx in 0..2 {
                        let sx = px + dx;
                        let sy = py + dy;
                        if sx < fb_width && sy < fb_height {
                            buffer[sy * fb_width + sx] = color;
                        }
                    }
                }
            }
        }
    }
}

pub fn draw_text_2x(
    buffer: &mut [u32],
    fb_width: usize,
    x: usize,
    y: usize,
    text: &str,
    color: u32,
) {
    let mut cx = x;
    for ch in text.chars() {
        draw_char_2x(buffer, fb_width, cx, y, ch, color);
        cx += 12; // (5 px * 2) + 2 px spacing
    }
}

fn glyph_rows(c: char) -> Option<&'static [u8; 7]> {
    match c.to_ascii_uppercase() {
        '0' => Some(&[
            0b01110,
            0b10001,
            0b10011,
            0b10101,
            0b11001,
            0b10001,
            0b01110,
        ]),
        '1' => Some(&[
            0b00100,
            0b01100,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
            0b01110,
        ]),
        '2' => Some(&[
            0b01110,
            0b10001,
            0b00001,
            0b00010,
            0b00100,
            0b01000,
            0b11111,
        ]),
        '3' => Some(&[
            0b11110,
            0b00001,
            0b00001,
            0b01110,
            0b00001,
            0b00001,
            0b11110,
        ]),
        '4' => Some(&[
            0b00010,
            0b00110,
            0b01010,
            0b10010,
            0b11111,
            0b00010,
            0b00010,
        ]),
        '5' => Some(&[
            0b11111,
            0b10000,
            0b10000,
            0b11110,
            0b00001,
            0b00001,
            0b11110,
        ]),
        '6' => Some(&[
            0b00110,
            0b01000,
            0b10000,
            0b11110,
            0b10001,
            0b10001,
            0b01110,
        ]),
        '7' => Some(&[
            0b11111,
            0b00001,
            0b00010,
            0b00100,
            0b01000,
            0b01000,
            0b01000,
        ]),
        '8' => Some(&[
            0b01110,
            0b10001,
            0b10001,
            0b01110,
            0b10001,
            0b10001,
            0b01110,
        ]),
        '9' => Some(&[
            0b01110,
            0b10001,
            0b10001,
            0b01111,
            0b00001,
            0b00010,
            0b11100,
        ]),
        'A' => Some(&[
            0b01110,
            0b10001,
            0b10001,
            0b11111,
            0b10001,
            0b10001,
            0b10001,
        ]),
        'B' => Some(&[
            0b11110,
            0b10001,
            0b10001,
            0b11110,
            0b10001,
            0b10001,
            0b11110,
        ]),
        'C' => Some(&[
            0b01110,
            0b10001,
            0b10000,
            0b10000,
            0b10000,
            0b10001,
            0b01110,
        ]),
        'D' => Some(&[
            0b11110,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b11110,
        ]),
        'E' => Some(&[
            0b11111,
            0b10000,
            0b10000,
            0b11110,
            0b10000,
            0b10000,
            0b11111,
        ]),
        'F' => Some(&[
            0b11111,
            0b10000,
            0b10000,
            0b11110,
            0b10000,
            0b10000,
            0b10000,
        ]),
        'G' => Some(&[
            0b01110,
            0b10001,
            0b10000,
            0b10111,
            0b10001,
            0b10001,
            0b01110,
        ]),
        'H' => Some(&[
            0b10001,
            0b10001,
            0b10001,
            0b11111,
            0b10001,
            0b10001,
            0b10001,
        ]),
        'I' => Some(&[
            0b01110,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
            0b01110,
        ]),
        'J' => Some(&[
            0b00001,
            0b00001,
            0b00001,
            0b00001,
            0b10001,
            0b10001,
            0b01110,
        ]),
        'K' => Some(&[
            0b10001,
            0b10010,
            0b10100,
            0b11000,
            0b10100,
            0b10010,
            0b10001,
        ]),
        'L' => Some(&[
            0b10000,
            0b10000,
            0b10000,
            0b10000,
            0b10000,
            0b10000,
            0b11111,
        ]),
        'M' => Some(&[
            0b10001,
            0b11011,
            0b10101,
            0b10101,
            0b10001,
            0b10001,
            0b10001,
        ]),
        'N' => Some(&[
            0b10001,
            0b11001,
            0b10101,
            0b10011,
            0b10001,
            0b10001,
            0b10001,
        ]),
        'O' => Some(&[
            0b01110,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b01110,
        ]),
        'P' => Some(&[
            0b11110,
            0b10001,
            0b10001,
            0b11110,
            0b10000,
            0b10000,
            0b10000,
        ]),
        'Q' => Some(&[
            0b01110,
            0b10001,
            0b10001,
            0b10001,
            0b10101,
            0b10010,
            0b01101,
        ]),
        'R' => Some(&[
            0b11110,
            0b10001,
            0b10001,
            0b11110,
            0b10100,
            0b10010,
            0b10001,
        ]),
        'S' => Some(&[
            0b01111,
            0b10000,
            0b10000,
            0b01110,
            0b00001,
            0b00001,
            0b11110,
        ]),
        'T' => Some(&[
            0b11111,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
        ]),
        'U' => Some(&[
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b01110,
        ]),
        'V' => Some(&[
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b10001,
            0b01010,
            0b00100,
        ]),
        'W' => Some(&[
            0b10001,
            0b10001,
            0b10001,
            0b10101,
            0b10101,
            0b10101,
            0b01010,
        ]),
        'X' => Some(&[
            0b10001,
            0b10001,
            0b01010,
            0b00100,
            0b01010,
            0b10001,
            0b10001,
        ]),
        'Y' => Some(&[
            0b10001,
            0b10001,
            0b01010,
            0b00100,
            0b00100,
            0b00100,
            0b00100,
        ]),
        'Z' => Some(&[
            0b11111,
            0b00001,
            0b00010,
            0b00100,
            0b01000,
            0b10000,
            0b11111,
        ]),
        ':' => Some(&[
            0b00000,
            0b00100,
            0b00100,
            0b00000,
            0b00100,
            0b00100,
            0b00000,
        ]),
        '.' => Some(&[
            0b00000,
            0b00000,
            0b00000,
            0b00000,
            0b00000,
            0b00110,
            0b00110,
        ]),
        '-' => Some(&[
            0b00000,
            0b00000,
            0b00000,
            0b11111,
            0b00000,
            0b00000,
            0b00000,
        ]),
        '+' => Some(&[
            0b00000,
            0b00100,
            0b00100,
            0b11111,
            0b00100,
            0b00100,
            0b00000,
        ]),
        '/' => Some(&[
            0b00001,
            0b00010,
            0b00100,
            0b01000,
            0b10000,
            0b00000,
            0b00000,
        ]),
        ' ' => Some(&[
            0b00000,
            0b00000,
            0b00000,
            0b00000,
            0b00000,
            0b00000,
            0b00000,
        ]),
        _ => None,
    }
}
