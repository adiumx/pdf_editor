//! Files whose `/Encrypt` is a dictionary written in the trailer (MuPDF writes them so) instead of
//! a reference to an object, which is also what the specification allows. The parser we use only
//! reads the second form, so such a file is first rewritten as an equivalent one: the dictionary
//! becomes an indirect object appended as an incremental update, with a new trailer that points
//! to it. Only files with a classic cross-reference table (`trailer`) are handled.

/// Index just past the PDF object that starts at `i` (after white space), or `None`.
fn skip_object(b: &[u8], mut i: usize) -> Option<usize> {
    let ws = |c: u8| matches!(c, b' ' | b'\n' | b'\r' | b'\t' | 0 | 12);
    let delimiter = |c: u8| ws(c) || matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%');
    while i < b.len() && ws(b[i]) {
        i += 1;
    }
    match *b.get(i)? {
        b'<' if b.get(i + 1) == Some(&b'<') => {
            i += 2;
            loop {
                while i < b.len() && ws(b[i]) {
                    i += 1;
                }
                if b.get(i..i + 2) == Some(b">>") {
                    return Some(i + 2);
                }
                i = skip_object(b, i)?; // the key
                i = skip_object(b, i)?; // its value
            }
        }
        b'<' => Some(b[i..].iter().position(|c| *c == b'>')? + i + 1),
        b'[' => {
            i += 1;
            loop {
                while i < b.len() && ws(b[i]) {
                    i += 1;
                }
                if *b.get(i)? == b']' {
                    return Some(i + 1);
                }
                i = skip_object(b, i)?;
            }
        }
        b'(' => {
            let (mut depth, mut k) = (0usize, i);
            while k < b.len() {
                match b[k] {
                    b'\\' => k += 1,
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(k + 1);
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            None
        }
        b'/' => {
            i += 1;
            while i < b.len() && !delimiter(b[i]) {
                i += 1;
            }
            Some(i)
        }
        _ => {
            let token_end = |mut k: usize| {
                while k < b.len() && !delimiter(b[k]) {
                    k += 1;
                }
                k
            };
            let mut end = token_end(i);
            if end == i {
                return None;
            }
            // `12 0 R`: an integer, an integer and the letter R are one reference.
            if b[i..end].iter().all(u8::is_ascii_digit) {
                let mut k = end;
                while k < b.len() && ws(b[k]) {
                    k += 1;
                }
                let second = token_end(k);
                if second > k && b[k..second].iter().all(u8::is_ascii_digit) {
                    let mut m = second;
                    while m < b.len() && ws(b[m]) {
                        m += 1;
                    }
                    if b.get(m) == Some(&b'R') && !b.get(m + 1).is_some_and(|c| c.is_ascii_alphanumeric()) {
                        end = m + 1;
                    }
                }
            }
            Some(end)
        }
    }
}

/// The file with its inline `/Encrypt` made an indirect object, or `None` when the file does not
/// have that form (or is not one this can rewrite).
pub(crate) fn make_encrypt_indirect(data: &[u8]) -> Option<Vec<u8>> {
    let trailer_at = data.windows(7).rposition(|w| w == b"trailer")?;
    let start = data[trailer_at + 7..].iter().position(|c| !c.is_ascii_whitespace())? + trailer_at + 7;
    if data.get(start..start + 2) != Some(b"<<") {
        return None;
    }
    let end = skip_object(data, start)?;
    let startxref = data[end..].windows(9).position(|w| w == b"startxref")? + end + 9;
    let prev: usize = String::from_utf8_lossy(&data[startxref..]).split_whitespace().next()?.parse().ok()?;

    // The trailer's entries.
    let mut entries: Vec<(&[u8], &[u8])> = Vec::new();
    let mut i = start + 2;
    loop {
        while i < end && data[i].is_ascii_whitespace() {
            i += 1;
        }
        if data.get(i..i + 2) == Some(b">>") {
            break;
        }
        let key_end = skip_object(data, i)?;
        let value_end = skip_object(data, key_end)?;
        let value_start = key_end + data[key_end..value_end].iter().position(|c| !c.is_ascii_whitespace())?;
        entries.push((&data[i..key_end], &data[value_start..value_end]));
        i = value_end;
    }
    let get = |key: &[u8]| entries.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let encrypt = get(b"/Encrypt").filter(|v| v.starts_with(b"<<"))?;
    let size: u32 = String::from_utf8_lossy(get(b"/Size")?).trim().parse().ok()?;
    let root = get(b"/Root")?;

    let mut out = data.to_vec();
    out.push(b'\n');
    let object_at = out.len();
    out.extend(format!("{size} 0 obj\n").bytes());
    out.extend(encrypt);
    out.extend(b"\nendobj\n");
    let xref_at = out.len();
    out.extend(format!("xref\n{size} 1\n{object_at:010} 00000 n \n").bytes());
    out.extend(format!("trailer\n<</Size {}/Root ", size + 1).bytes());
    out.extend(root);
    for key in [&b"/Info"[..], b"/ID"] {
        if let Some(value) = get(key) {
            out.extend(key);
            out.push(b' ');
            out.extend(value);
        }
    }
    out.extend(format!("/Encrypt {size} 0 R/Prev {prev}>>\nstartxref\n{xref_at}\n%%EOF\n").bytes());
    Some(out)
}
