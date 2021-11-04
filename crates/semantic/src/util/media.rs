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
            jpegtran(data)
        }
        "image/png" => oxipng(data),
        other => Err(anyhow::anyhow!("Unsupported mime type: {}", other)),
    }
}

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

fn jpegtran(data: &[u8]) -> Result<Vec<u8>, AnyError> {
    let mut child = Command::new("jpegtran")
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
