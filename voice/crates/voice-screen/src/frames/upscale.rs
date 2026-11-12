//! Pixel scaling: twice the size for the text readers, which read small text better that way,
//! and a downscale for a frame larger than the JPEG or the video allows.

use crate::platform::Pixels;

/// Frames at most this high are read at twice their size.
pub const UPSCALE_MAX_HEIGHT: u32 = 1080;

/// Twice the size, bilinear: each new pixel between two old ones is their mean. Each row is
/// doubled across once, then each new row is the mean of the doubled rows around it.
pub fn upscale_2x(pixels: &Pixels) -> Pixels {
    let (w, h) = (pixels.width as usize, pixels.height as usize);
    let row_len = w * 4;
    let wide = |y: usize| -> Vec<u8> {
        let row = &pixels.bgra[y * row_len..(y + 1) * row_len];
        let mut out = Vec::with_capacity(row_len * 2);
        for (x, px) in row.chunks_exact(4).enumerate() {
            let right = if x + 1 < w {
                &row[(x + 1) * 4..(x + 2) * 4]
            } else {
                px
            };
            out.extend_from_slice(px);
            out.extend(px.iter().zip(right).map(|(a, b)| mean(*a, *b)));
        }
        out
    };
    let mut out = Vec::with_capacity(row_len * 4 * h);
    let mut current = wide(0);
    for y in 0..h {
        let next = if y + 1 < h {
            wide(y + 1)
        } else {
            current.clone()
        };
        out.extend_from_slice(&current);
        out.extend(current.iter().zip(&next).map(|(a, b)| mean(*a, *b)));
        current = next;
    }
    Pixels {
        bgra: out,
        width: (w * 2) as u32,
        height: (h * 2) as u32,
    }
}

/// Scales BGRA pixels to `width` x `height`, averaging each output pixel's block of source
/// pixels. Used for the JPEG's maximum side and for a monitor larger than the video encoder
/// takes; both only ever shrink a frame.
pub fn scale_to(pixels: &Pixels, width: u32, height: u32) -> Pixels {
    let (sw, sh) = (pixels.width as usize, pixels.height as usize);
    let (dw, dh) = (width as usize, height as usize);
    let mut bgra = vec![0u8; dw * dh * 4];
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return Pixels {
            bgra,
            width,
            height,
        };
    }
    for y in 0..dh {
        let top = (y * sh / dh).min(sh - 1);
        let bottom = ((y + 1) * sh / dh).max(top + 1).min(sh);
        for x in 0..dw {
            let left = (x * sw / dw).min(sw - 1);
            let right = ((x + 1) * sw / dw).max(left + 1).min(sw);
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for row in top..bottom {
                let start = (row * sw + left) * 4;
                let end = (row * sw + right) * 4;
                for px in pixels.bgra[start..end].chunks_exact(4) {
                    for (channel, value) in sum.iter_mut().zip(px) {
                        *channel += u32::from(*value);
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            let out = (y * dw + x) * 4;
            for channel in 0..4 {
                bgra[out + channel] = ((sum[channel] + count / 2) / count) as u8;
            }
        }
    }
    Pixels {
        bgra,
        width,
        height,
    }
}

fn mean(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b)) / 2) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_down_averages_blocks() {
        let white_black = Pixels {
            bgra: vec![
                255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255,
            ],
            width: 2,
            height: 2,
        };
        let small = scale_to(&white_black, 1, 1);
        assert_eq!((small.width, small.height), (1, 1));
        assert_eq!(small.bgra, vec![128, 128, 128, 255]);
        let wide = scale_to(&white_black, 1, 2);
        assert_eq!(wide.bgra.len(), 2 * 4);
        assert_eq!(wide.bgra[..4], vec![128, 128, 128, 255]);
        assert_eq!(wide.bgra[4..], vec![128, 128, 128, 255]);
    }

    #[test]
    fn upscales_between_pixels() {
        let pixels = Pixels {
            bgra: vec![0, 0, 0, 255, 200, 200, 200, 255],
            width: 2,
            height: 1,
        };
        let big = upscale_2x(&pixels);
        assert_eq!((big.width, big.height), (4, 2));
        let blue: Vec<u8> = big.bgra.chunks(4).map(|p| p[0]).collect();
        assert_eq!(blue, vec![0, 100, 200, 200, 0, 100, 200, 200]);
        assert!(big.bgra.chunks(4).all(|p| p[3] == 255));

        // Down a column, and the corner between four pixels.
        let square = Pixels {
            bgra: [0u8, 40, 80, 120]
                .iter()
                .flat_map(|&v| [v, v, v, 255])
                .collect(),
            width: 2,
            height: 2,
        };
        let big = upscale_2x(&square);
        assert_eq!((big.width, big.height), (4, 4));
        let blue: Vec<u8> = big.bgra.chunks(4).map(|p| p[0]).collect();
        assert_eq!(
            blue,
            vec![0, 20, 40, 40, 40, 60, 80, 80, 80, 100, 120, 120, 80, 100, 120, 120]
        );
    }
}
