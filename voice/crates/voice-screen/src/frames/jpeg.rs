//! A saved frame's pixels, its JPEG through WinRT's `BitmapEncoder` (WIC underneath, 3.5 to
//! 6.7 ms for 1080p, no extra crate) and the way back for frames a crash left without text.

use voice_screen_files::frames::{JPEG_MAX_SIDE, JPEG_QUALITY};
use windows::core::h;
use windows::Foundation::{PropertyType, PropertyValue};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapEncoder, BitmapInterpolationMode, BitmapPixelFormat,
    BitmapPropertySet, BitmapTypedValue, SoftwareBitmap,
};
use windows::Storage::Streams::{DataReader, DataWriter, InMemoryRandomAccessStream};

use crate::capture::d3d::D3D;
use crate::capture::CapturedFrame;
use crate::platform::Pixels;

/// Reads a captured frame back from the GPU.
pub fn pixels(d3d: &D3D, frame: &CapturedFrame) -> windows::core::Result<Pixels> {
    Ok(Pixels {
        bgra: d3d.read_bgra(&frame.picture.0, frame.width, frame.height)?,
        width: frame.width,
        height: frame.height,
    })
}

/// A software bitmap of the pixels, opaque.
pub fn bitmap(pixels: &Pixels) -> windows::core::Result<SoftwareBitmap> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(&pixels.bgra)?;
    let buffer = writer.DetachBuffer()?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        pixels.width as i32,
        pixels.height as i32,
        BitmapAlphaMode::Ignore,
    )
}

/// The size a JPEG gets: the frame's own, unless its longest side is over `JPEG_MAX_SIDE`.
pub fn jpeg_size(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= JPEG_MAX_SIDE {
        return (width, height);
    }
    let scale = |side: u32| {
        ((u64::from(side) * u64::from(JPEG_MAX_SIDE) / u64::from(longest)) as u32).max(1)
    };
    (scale(width), scale(height))
}

/// The JPEG at quality `JPEG_QUALITY`, and its width and height.
pub fn encode(bitmap: &SoftwareBitmap) -> windows::core::Result<(Vec<u8>, u32, u32)> {
    let (width, height) = (bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32);
    let stream = InMemoryRandomAccessStream::new()?;
    let options = BitmapPropertySet::new()?;
    options.Insert(
        h!("ImageQuality"),
        &BitmapTypedValue::Create(
            &PropertyValue::CreateSingle(JPEG_QUALITY)?,
            PropertyType::Single,
        )?,
    )?;
    let encoder = BitmapEncoder::CreateWithEncodingOptionsAsync(
        BitmapEncoder::JpegEncoderId()?,
        &stream,
        &options,
    )?
    .join()?;
    encoder.SetSoftwareBitmap(bitmap)?;
    let (out_width, out_height) = jpeg_size(width, height);
    if (out_width, out_height) != (width, height) {
        let transform = encoder.BitmapTransform()?;
        transform.SetScaledWidth(out_width)?;
        transform.SetScaledHeight(out_height)?;
        transform.SetInterpolationMode(BitmapInterpolationMode::Fant)?;
    }
    encoder.FlushAsync()?.join()?;
    let size = stream.Size()? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
    reader.LoadAsync(size)?.join()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok((bytes, out_width, out_height))
}

/// Decodes an image file's bytes, as BGRA.
pub fn decode(bytes: &[u8]) -> windows::core::Result<SoftwareBitmap> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
    decoder
        .GetSoftwareBitmapConvertedAsync(BitmapPixelFormat::Bgra8, BitmapAlphaMode::Ignore)?
        .join()
}

/// The pixel size of an image file, through `BitmapDecoder`.
pub fn decode_size(bytes: &[u8]) -> windows::core::Result<(u32, u32)> {
    let bitmap = decode(bytes)?;
    Ok((bitmap.PixelWidth()? as u32, bitmap.PixelHeight()? as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_longest_side_at_most_2560() {
        assert_eq!(jpeg_size(1920, 1080), (1920, 1080));
        assert_eq!(jpeg_size(1440, 2560), (1440, 2560));
        assert_eq!(jpeg_size(3840, 2160), (2560, 1440));
        assert_eq!(jpeg_size(5120, 1440), (2560, 720));
    }

    #[test]
    fn encodes_and_decodes_a_jpeg() {
        crate::com::init_mta();
        let (width, height) = (320u32, 200u32);
        let mut bgra = vec![255u8; (width * height * 4) as usize];
        for px in bgra.chunks_exact_mut(4).take(5000) {
            px[..3].copy_from_slice(&[40, 80, 160]);
        }
        let pixels = Pixels {
            bgra,
            width,
            height,
        };
        let (bytes, w, h) = encode(&bitmap(&pixels).unwrap()).unwrap();
        assert_eq!((w, h), (width, height));
        assert_eq!(&bytes[..2], &[0xFF, 0xD8], "a JPEG starts with SOI");
        assert_eq!(decode_size(&bytes).unwrap(), (width, height));

        let big = Pixels {
            bgra: vec![200u8; 3000 * 100 * 4],
            width: 3000,
            height: 100,
        };
        let (bytes, w, h) = encode(&bitmap(&big).unwrap()).unwrap();
        assert_eq!((w, h), (2560, 85));
        assert_eq!(decode_size(&bytes).unwrap(), (2560, 85));
    }
}
