//! Soft-wrap a chat string into rows of character ranges.
//! Movement and width follow grapheme clusters, the same way the buffer does.
//! A newline starts a row. A row that is wider than `width` breaks at a space
//! when it has one, otherwise at the last cluster that fits.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

struct Cluster {
    start: usize,
    end: usize,
    width: usize,
    newline: bool,
    space: bool,
}

fn clusters(text: &str) -> Vec<Cluster> {
    let mut start = 0usize;
    text.graphemes(true)
        .map(|grapheme| {
            let end = start + grapheme.chars().count();
            let newline = grapheme == "\n" || grapheme == "\r" || grapheme == "\r\n";
            let cluster = Cluster {
                start,
                end,
                width: if newline { 0 } else { grapheme.width() },
                newline,
                space: grapheme == " ",
            };
            start = end;
            cluster
        })
        .collect()
}

/// `(start, end)` are character indexes into `text`. `end` is exclusive.
/// An empty string is one empty row so the prompt still has a place for the cursor.
pub fn rows(text: &str, width: usize) -> Vec<(usize, usize)> {
    let clusters = clusters(text);
    if clusters.is_empty() {
        return vec![(0, 0)];
    }
    let width = width.max(1);
    let mut out = Vec::new();
    let mut index = 0;
    while index < clusters.len() {
        if clusters[index].newline {
            out.push((clusters[index].start, clusters[index].start));
            index += 1;
            continue;
        }
        let row_start = clusters[index].start;
        let mut used = 0usize;
        let mut last_space = None;
        let mut taken = 0usize;
        while index < clusters.len() && !clusters[index].newline {
            let cell = clusters[index].width;
            if used + cell > width && taken > 0 {
                break;
            }
            if clusters[index].space {
                last_space = Some(index);
            }
            used += clusters[index].width;
            taken += 1;
            index += 1;
            if used >= width {
                break;
            }
        }
        if index < clusters.len() && !clusters[index].newline {
            if let Some(space) = last_space.filter(|space| clusters[*space].start > row_start) {
                out.push((row_start, clusters[space].start));
                index = space + 1;
                continue;
            }
        }
        let end = clusters
            .get(index.wrapping_sub(1))
            .map(|c| c.end)
            .unwrap_or(row_start);
        out.push((row_start, end));
        if index < clusters.len() && clusters[index].newline {
            index += 1;
        }
    }
    if text.ends_with('\n') || text.ends_with('\r') {
        let len = text.chars().count();
        out.push((len, len));
    }
    out
}

/// Character index of the next grapheme after `cursor`, or the end of the current one.
pub fn next_boundary(text: &str, cursor: usize) -> usize {
    let len = text.chars().count();
    let cursor = cursor.min(len);
    clusters(text)
        .into_iter()
        .map(|cluster| cluster.end)
        .find(|end| *end > cursor)
        .unwrap_or(len)
}

/// Character index of the grapheme before `cursor`.
pub fn prev_boundary(text: &str, cursor: usize) -> usize {
    let cursor = cursor.min(text.chars().count());
    clusters(text)
        .into_iter()
        .map(|cluster| cluster.start)
        .take_while(|start| *start < cursor)
        .last()
        .unwrap_or(0)
}

/// Character index on `row` closest to visual column `column`, on a grapheme edge.
pub fn index_at(text: &str, row: (usize, usize), column: usize) -> usize {
    let mut used = 0usize;
    let mut at = row.0;
    for cluster in clusters(text) {
        if cluster.end <= row.0 || cluster.newline {
            continue;
        }
        if cluster.start >= row.1 {
            break;
        }
        if used + cluster.width > column {
            break;
        }
        used += cluster.width;
        at = cluster.end.min(row.1);
    }
    at
}

/// Visual column of `cursor` within a row that starts at `start`.
pub fn column_at(text: &str, start: usize, cursor: usize) -> usize {
    let mut width = 0usize;
    for cluster in clusters(text) {
        if cluster.newline || cluster.end <= start || cluster.start >= cursor {
            continue;
        }
        if cluster.start < start {
            continue;
        }
        if cluster.start >= cursor {
            break;
        }
        width += cluster.width;
    }
    width
}

#[cfg(test)]
mod tests {
    use super::{column_at, next_boundary, prev_boundary, rows};

    #[test]
    fn a_wide_line_wraps_and_a_newline_starts_a_row() {
        let wrapped = rows("hello world", 8);
        assert_eq!(wrapped, vec![(0, 5), (6, 11)]);
        let broken = rows("ab\ncd", 8);
        assert_eq!(broken, vec![(0, 2), (3, 5)]);
    }

    #[test]
    fn a_thai_cluster_is_one_cell_and_one_step() {
        let word = "ที่";
        assert_eq!(word.chars().count(), 3);
        assert_eq!(column_at(word, 0, 3), 1);
        assert_eq!(next_boundary(word, 0), 3);
        assert_eq!(prev_boundary(word, 3), 0);
        assert_eq!(prev_boundary(word, 2), 0);
    }
}
