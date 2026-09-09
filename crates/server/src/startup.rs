use std::io::{self, BufRead, IsTerminal, Write};

/// Read a logfs password at CLI startup. Library construction never prompts.
pub fn prompt_blob_password(blob_uri: &str) -> io::Result<Option<String>> {
    if !crate::is_logfs_blob_uri(blob_uri) {
        return Ok(None);
    }
    eprint!("Logfs blob-store password (leave empty for no password protection): ");
    io::stderr().flush()?;
    let password = if io::stdin().is_terminal() {
        rpassword::read_password()?
    } else {
        read_password_line(&mut io::stdin().lock())?
    };
    if password.is_empty() {
        eprintln!(
            "No logfs password entered; opening the blob store without password protection (unencrypted)."
        );
        Ok(None)
    } else {
        Ok(Some(password))
    }
}

fn read_password_line(input: &mut impl BufRead) -> io::Result<String> {
    let mut password = String::new();
    if input.read_line(&mut password)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "no logfs password input received; supply a password or an empty line",
        ));
    }
    if password.ends_with('\n') {
        password.pop();
        if password.ends_with('\r') {
            password.pop();
        }
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_input_preserves_spaces_and_consumes_one_line() {
        let mut input = io::Cursor::new(b"  password  \r\nnext\n");
        assert_eq!(read_password_line(&mut input).unwrap(), "  password  ");
        assert_eq!(read_password_line(&mut input).unwrap(), "next");
        assert_eq!(
            read_password_line(&mut input).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }

    #[test]
    fn blank_line_explicitly_selects_no_password() {
        assert_eq!(read_password_line(&mut io::Cursor::new(b"\n")).unwrap(), "");
        assert_eq!(
            read_password_line(&mut io::Cursor::new(b"\r\n")).unwrap(),
            ""
        );
    }
}
