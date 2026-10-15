//! Cutting a TrueType font down to the glyphs a document uses.
//!
//! A bundled font is 300-800 KB and a document only ever draws a few letters of it. Glyph numbers
//! are kept (the document's width table and text map are written in them), the outlines of the
//! glyphs that are not used are emptied, and the tables nothing in a PDF needs are dropped; what is
//! left compresses to a few kilobytes.

use std::collections::BTreeSet;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

/// Glyphs a composite glyph is made of.
fn components(glyph: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    if glyph.len() < 10 || i16::from_be_bytes([glyph[0], glyph[1]]) >= 0 {
        return out;
    }
    let mut at = 10;
    loop {
        let (Some(flags), Some(index)) = (u16_at(glyph, at), u16_at(glyph, at + 2)) else { break };
        out.push(index);
        at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
        at += if flags & 0x0008 != 0 {
            2
        } else if flags & 0x0040 != 0 {
            4
        } else if flags & 0x0080 != 0 {
            8
        } else {
            0
        };
        if flags & 0x0020 == 0 {
            break;
        }
    }
    out
}

/// A minimal `cmap` (format 4, no characters): some readers insist on finding one.
fn empty_cmap() -> Vec<u8> {
    let mut t = vec![0, 0, 0, 1, 0, 3, 0, 1, 0, 0, 0, 12];
    t.extend([0, 4, 0, 24, 0, 0, 0, 2, 0, 2, 0, 0, 0, 0, 0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 1, 0, 0]);
    t
}

/// The font cut down to `keep` (and glyph 0), or `None` when it is not a plain TrueType font with
/// outlines in a `glyf` table.
pub(crate) fn subset(data: &[u8], keep: &BTreeSet<u16>) -> Option<Vec<u8>> {
    if !matches!(u32_at(data, 0)?, 0x0001_0000 | 0x7472_7565) {
        return None;
    }
    let count = u16_at(data, 4)? as usize;
    let mut tables: Vec<([u8; 4], &[u8])> = Vec::new();
    for i in 0..count {
        let rec = 12 + 16 * i;
        let tag: [u8; 4] = data.get(rec..rec + 4)?.try_into().ok()?;
        let (offset, length) = (u32_at(data, rec + 8)? as usize, u32_at(data, rec + 12)? as usize);
        tables.push((tag, data.get(offset..offset.checked_add(length)?)?));
    }
    let table = |tag: &[u8; 4]| tables.iter().find(|(t, _)| t == tag).map(|(_, d)| *d);
    let (head, hhea, maxp, hmtx, loca, glyf) = (table(b"head")?, table(b"hhea")?, table(b"maxp")?, table(b"hmtx")?, table(b"loca")?, table(b"glyf")?);
    let glyph_count = u16_at(maxp, 4)? as usize;
    let long = i16::from_be_bytes([*head.get(50)?, *head.get(51)?]) != 0;
    let offset_of = |gid: usize| -> Option<usize> {
        if long { Some(u32_at(loca, gid * 4)? as usize) } else { Some(u16_at(loca, gid * 2)? as usize * 2) }
    };
    let glyph = |gid: usize| -> Option<&[u8]> { glyf.get(offset_of(gid)?..offset_of(gid + 1)?) };

    // The glyphs to keep, with whatever the composite ones are built from.
    let mut wanted: BTreeSet<u16> = keep.iter().copied().filter(|g| (*g as usize) < glyph_count).collect();
    wanted.insert(0);
    let mut queue: Vec<u16> = wanted.iter().copied().collect();
    while let Some(g) = queue.pop() {
        for c in components(glyph(g as usize)?) {
            if (c as usize) < glyph_count && wanted.insert(c) {
                queue.push(c);
            }
        }
    }

    let mut new_glyf: Vec<u8> = Vec::new();
    let mut new_loca: Vec<u8> = Vec::with_capacity((glyph_count + 1) * 4);
    for gid in 0..glyph_count {
        new_loca.extend((new_glyf.len() as u32).to_be_bytes());
        if wanted.contains(&(gid as u16)) {
            new_glyf.extend(glyph(gid)?);
            while new_glyf.len() % 4 != 0 {
                new_glyf.push(0);
            }
        }
    }
    new_loca.extend((new_glyf.len() as u32).to_be_bytes());

    let mut new_head = head.to_vec();
    new_head.get_mut(8..12)?.copy_from_slice(&[0; 4]); // checkSumAdjustment, set below
    new_head.get_mut(50..52)?.copy_from_slice(&1i16.to_be_bytes());
    let mut post = table(b"post").map(<[u8]>::to_vec).unwrap_or_else(|| vec![0; 32]);
    post.truncate(32);
    post.resize(32, 0);
    post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());

    let mut out_tables: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"cmap", empty_cmap()),
        (*b"glyf", new_glyf),
        (*b"head", new_head),
        (*b"hhea", hhea.to_vec()),
        (*b"hmtx", hmtx.to_vec()),
        (*b"loca", new_loca),
        (*b"maxp", maxp.to_vec()),
        (*b"post", post),
    ];
    for tag in [b"cvt ", b"fpgm", b"prep", b"OS/2"] {
        if let Some(t) = table(tag) {
            out_tables.push((*tag, t.to_vec()));
        }
    }
    out_tables.sort_by_key(|(tag, _)| *tag);

    let n = out_tables.len();
    let mut entry_selector = 0u16;
    while (1usize << (entry_selector + 1)) <= n {
        entry_selector += 1;
    }
    let search_range = (1u16 << entry_selector) * 16;
    let mut out: Vec<u8> = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend((n as u16).to_be_bytes());
    out.extend(search_range.to_be_bytes());
    out.extend(entry_selector.to_be_bytes());
    out.extend((n as u16 * 16 - search_range).to_be_bytes());
    let mut offset = 12 + 16 * n;
    let mut head_at = 0;
    for (tag, body) in &out_tables {
        out.extend(tag);
        out.extend(checksum(body).to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((body.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = offset;
        }
        offset += body.len().div_ceil(4) * 4;
    }
    for (_, body) in &out_tables {
        out.extend(body);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
    out[head_at + 8..head_at + 12].copy_from_slice(&adjustment.to_be_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ttf_parser::Face;

    #[test]
    fn a_subset_keeps_the_outlines_it_was_asked_for_and_is_small() {
        let data = include_bytes!("../fonts/Carlito-Regular.ttf");
        let face = Face::parse(data, 0).unwrap();
        let wanted: Vec<char> = "Holañ".chars().collect();
        let gids: BTreeSet<u16> = wanted.iter().map(|c| face.glyph_index(*c).unwrap().0).collect();
        let small = subset(data, &gids).expect("subsets");
        assert!(small.len() < data.len() / 8, "{} of {}", small.len(), data.len());
        let cut = Face::parse(&small, 0).expect("still a font");
        for c in wanted {
            let g = face.glyph_index(c).unwrap();
            assert_eq!(cut.glyph_bounding_box(g), face.glyph_bounding_box(g), "{c}");
            assert_eq!(cut.glyph_hor_advance(g), face.glyph_hor_advance(g), "{c}");
        }
        let other = face.glyph_index('Z').unwrap();
        assert_eq!(cut.glyph_bounding_box(other), None);
        assert_eq!(cut.units_per_em(), face.units_per_em());
    }

    #[test]
    fn composite_glyphs_bring_their_parts() {
        // Accented letters of Liberation are usually built from a base and a mark.
        let data = include_bytes!("../fonts/LiberationSans-Regular.ttf");
        let face = Face::parse(data, 0).unwrap();
        let g = face.glyph_index('ñ').unwrap();
        let small = subset(data, &BTreeSet::from([g.0])).unwrap();
        let cut = Face::parse(&small, 0).unwrap();
        assert_eq!(cut.glyph_bounding_box(g), face.glyph_bounding_box(g));
    }

    #[test]
    fn what_is_not_truetype_is_left_alone() {
        assert!(subset(b"OTTO....", &BTreeSet::new()).is_none());
        assert!(subset(b"", &BTreeSet::new()).is_none());
    }
}
