use crate::types::{Converted, Image};
use crate::{Budget, ConversionError, Settings};
use std::io::Read;

type Dispatch = fn(&[u8], Settings, &mut Budget) -> Result<Converted, ConversionError>;

// Hard limits applied to every archive, on top of the shared decompression budget
// and nesting-depth cap carried by `Budget`. These bound the work a single
// malicious zip can demand regardless of what its central directory claims.
const MAX_ENTRIES: usize = 4096; // refuse to even iterate beyond this many members
const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB decompressed per member
const MAX_RATIO: u64 = 1000; // declared-uncompressed : compressed ceiling
const RATIO_FLOOR: u64 = 1024 * 1024; // only ratio-check members declaring > 1 MiB

// A plain ZIP (not an OOXML/xlsb package) is expanded entry by entry: each member
// is re-dispatched through the same router, its markdown concatenated under an `##`
// heading and its images namespaced by entry path.
//
// Zip-bomb defenses, in order of authority:
//  1. entry-count cap        — never iterate more than MAX_ENTRIES members;
//  2. declared-size pre-check — skip members whose central-directory size, or
//     uncompressed:compressed ratio, is already implausible (cheap, pre-read);
//  3. shared byte budget      — the whole nested-archive tree may only ever
//     decompress MAX_TOTAL_DECOMPRESSED bytes in total;
//  4. bounded read            — decompression is hard-limited with `Read::take`, so
//     a member that lies about its size in the directory still cannot expand past
//     the cap (the actual bytes read are the source of truth);
//  5. depth cap               — carried by `Budget`, bounds nesting recursion.
// Anything skipped by a guard is listed in a trailing note; nothing is silent.
pub fn convert(
    blob: &[u8],
    settings: Settings,
    budget: &mut Budget,
    dispatch: Dispatch,
) -> Result<Converted, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(blob)).map_err(|e| e.to_string())?;

    let total_entries = zip.len();
    let limit = total_entries.min(MAX_ENTRIES);

    let mut markdown = String::new();
    let mut images: Vec<Image> = Vec::new();
    // Each skip records the member name and the reason, so a guard rejection is
    // distinguishable from a member that simply isn't a supported document.
    let mut skipped: Vec<(String, &'static str)> = Vec::new();

    for i in 0..limit {
        let mut file = match zip.by_index(i) {
            Ok(f) => f,
            Err(_) => continue,
        };
        if !file.is_file() {
            continue;
        }
        let name = file.name().to_string();
        let declared = file.size();
        let compressed = file.compressed_size();

        let implausible_ratio =
            compressed > 0 && declared > RATIO_FLOOR && declared / compressed > MAX_RATIO;
        // Cheap pre-read rejection from the central-directory metadata. Order matters
        // so each guard owns a distinct, testable reason.
        let pre_reason = if declared > MAX_ENTRY_BYTES {
            Some("too large")
        } else if implausible_ratio {
            Some("suspicious compression ratio")
        } else if declared > budget.remaining() {
            Some("exceeds remaining budget")
        } else {
            None
        };
        if let Some(reason) = pre_reason {
            skipped.push((name, reason));
            continue;
        }

        // Bounded read: cap at the smaller of the per-entry limit and the remaining
        // shared budget, plus one byte so an over-cap stream is detectable even when
        // the central directory under-reported the real size.
        let cap = MAX_ENTRY_BYTES.min(budget.remaining()).saturating_add(1);
        let mut bytes = Vec::new();
        if (&mut file).take(cap).read_to_end(&mut bytes).is_err() {
            skipped.push((name, "unreadable"));
            continue;
        }
        let read = bytes.len() as u64;
        if read > MAX_ENTRY_BYTES {
            skipped.push((name, "too large"));
            continue;
        }
        if !budget.charge(read) {
            skipped.push((name, "exceeds remaining budget"));
            continue;
        }
        drop(file);

        budget.enter();
        let result = dispatch(&bytes, settings, budget);
        budget.leave();

        match result {
            Ok(conv) => {
                markdown.push_str(&format!("## {name}\n\n"));
                markdown.push_str(conv.markdown.trim_end());
                markdown.push_str("\n\n");
                let prefix = name.replace('/', "_");
                for img in conv.images {
                    images.push(Image {
                        name: format!("{prefix}__{}", img.name),
                        data: img.data,
                    });
                }
            }
            Err(_) => skipped.push((name, "unsupported")),
        }
    }

    if !skipped.is_empty() {
        let plural = if skipped.len() == 1 { "y" } else { "ies" };
        let listed: Vec<String> = skipped
            .iter()
            .map(|(name, reason)| format!("{name} ({reason})"))
            .collect();
        markdown.push_str(&format!(
            "> Skipped {} entr{plural}: {}\n",
            skipped.len(),
            listed.join(", ")
        ));
    }

    if total_entries > MAX_ENTRIES {
        markdown.push_str(&format!(
            "> {} further entries beyond the {MAX_ENTRIES}-entry cap were not examined.\n",
            total_entries - MAX_ENTRIES
        ));
    }

    Ok(Converted {
        markdown: markdown.trim_end().to_string(),
        images,
    })
}
