use std::{
    io::{BufReader, BufWriter, Read, Write},
    process::{Command, Stdio},
};

use anyhow::{bail, Context};
use factordb::AnyError;

pub fn optimize_image_data(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let kind = infer::get(data).context("Could not determine mime type")?;
    match kind.mime_type() {
        "image/jpeg" => {
            // Try to use jpegtran binary
            mozjpeg_jpegtran(data)
        }
        "image/png" => oxipng(data),
        other => Err(anyhow::anyhow!("Unsupported mime type: {}", other)),
    }
}

/// Losslessly optimize pngs with oxipng as an embedded library.
#[cfg(feature = "media-optimizers")]
fn oxipng(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    oxipng::optimize_from_memory(data, &oxipng::Options::from_preset(4))
        .context("Could not optimize png with oxipng")
}

/// Losslessly optimize pngs with oxipng as a CLI tool.
#[cfg(not(feature = "media-optimizers"))]
fn oxipng(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let mut child = Command::new("oxipng")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(&["-o", "2", "-"])
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .context("Empty stdin")
        .map(BufWriter::new)?;
    let mut stdout = child
        .stdout
        .take()
        .context("Empty stdout")
        .map(BufReader::new)?;

    let reader = std::thread::spawn(move || -> Result<Vec<u8>, anyhow::Error> {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer)?;
        Ok(buffer)
    });

    stdin.write_all(data)?;
    std::mem::drop(stdin);

    let output = reader
        .join()
        .map_err(|err| anyhow::anyhow!("Reader failed {:?}", err))??;

    let status = child.wait()?;
    if !status.success() {
        bail!("oxipng failed");
    }

    Ok(output)
}

// /// Losslessly optimize jpegs with mozjpeg as a CLI tool.
// #[cfg(feature = "media-optimizers")]
// fn mozjpeg_jpegtran(data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
//     std::panic::catch_unwind(|| -> Result<Vec<u8>, anyhow::Error> {
//         let decomp = mozjpeg::Decompress::new_mem(data)?;

//         let mut compress = mozjpeg::Compress::new(decomp.color_space());
//         compress.set_raw_data_in(true);
//         compress.set_size(decomp.width(), decomp.height());
//         compress.set_optimize_scans(true);
//         compress.set_optimize_coding(true);
//         compress.set_mem_dest();

//         let mut bitmaps: Vec<_> = decomp
//             .components()
//             .iter()
//             .map(|_c| Vec::with_capacity(decomp.width() * decomp.height()))
//             .collect();

//         let mut decomp = decomp.raw()?;
//         {
//             let mut bitmap_refs: Vec<_> = bitmaps.iter_mut().collect();
//             decomp.read_raw_data(&mut bitmap_refs);
//             if !decomp.finish_decompress() {
//                 bail!("Could not decompress image")
//             }
//         }

//         let bitmap_refs: Vec<_> = bitmaps.iter().map(|v| v.as_slice()).collect();
//         if !compress.write_raw_data(&bitmap_refs) {
//             bail!("Could not compress image");
//         }

//         compress.finish_compress();

//         let data = compress
//             .data_to_vec()
//             .map_err(|_| anyhow!("Could not compress image"))?;

//         Ok(data)
//     })
//     .map_err(|_err| anyhow!("mozjpeg failed"))?
// }

/// Losslessly optimize jpegs with mozjpeg as a CLI tool.
// #[cfg(not(feature = "media-optimizers"))]
fn mozjpeg_jpegtran(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let mut child = Command::new("jpegtran")
        .arg("-optimize")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .context("Empty stdin")
        .map(BufWriter::new)?;
    let mut stdout = child
        .stdout
        .take()
        .context("Empty stdout")
        .map(BufReader::new)?;

    let reader = std::thread::spawn(move || -> Result<Vec<u8>, anyhow::Error> {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer)?;
        Ok(buffer)
    });

    stdin.write_all(data)?;
    std::mem::drop(stdin);

    let output = reader
        .join()
        .map_err(|err| anyhow::anyhow!("Reader failed {:?}", err))??;

    let status = child.wait()?;
    if !status.success() {
        bail!("oxipng failed");
    }

    Ok(output)
}
