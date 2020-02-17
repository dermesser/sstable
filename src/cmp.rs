use std::cmp::Ordering;

/// Comparator trait, supporting types that can be nested (i.e., add additional functionality on
/// top of an inner comparator)
pub trait Cmp {
    /// Compare to byte strings, bytewise.
    fn cmp(&self, &[u8], &[u8]) -> Ordering;

    /// Return the shortest byte string that compares "Greater" to the first argument and "Less" to
    /// the second one.
    fn find_shortest_sep(&self, &[u8], &[u8]) -> Vec<u8>;
    /// Return the shortest byte string that compares "Greater" to the argument.
    fn find_short_succ(&self, &[u8]) -> Vec<u8>;

    /// A unique identifier for a comparator. A comparator wrapper (like InternalKeyCmp) may
    /// return the id of its inner comparator.
    fn id(&self) -> &'static str;
}

/// Lexical comparator.
#[derive(Clone)]
pub struct DefaultCmp;

impl Cmp for DefaultCmp {
    fn cmp(&self, a: &[u8], b: &[u8]) -> Ordering {
        a.cmp(b)
    }

    fn id(&self) -> &'static str {
        "leveldb.BytewiseComparator"
    }

    fn find_shortest_sep(&self, a: &[u8], b: &[u8]) -> Vec<u8> {
        if a == b {
            return a.to_vec();
        }

        let min = if a.len() < b.len() { a.len() } else { b.len() };
        let mut diff_at = 0;

        while diff_at < min && a[diff_at] == b[diff_at] {
            diff_at += 1;
        }

        while diff_at < min {
            let diff = a[diff_at];
            if diff < 0xff && diff + 1 < b[diff_at] {
                let mut sep = Vec::from(&a[0..diff_at + 1]);
                sep[diff_at] += 1;
                assert!(self.cmp(&sep, b) == Ordering::Less);
                return sep;
            }

            diff_at += 1;
        }
        // Backup case: either `a` is full of 0xff, or all different places are less than 2
        // characters apart.
        // The result is not necessarily short, but a good separator: e.g., "abc" vs "abd" ->
        // "abc\0", which is greater than "abc" and lesser than "abd".
        let mut sep = Vec::with_capacity(a.len() + 1);
        sep.extend_from_slice(a);
        // Append a 0 byte; by making it longer than a, it will compare greater to it.
        sep.extend_from_slice(&[0]);
        return sep;
    }

    fn find_short_succ(&self, a: &[u8]) -> Vec<u8> {
        let mut result = a.to_vec();
        for i in 0..a.len() {
            if a[i] != 0xff {
                result[i] += 1;
                result.resize(i + 1, 0);
                return result;
            }
        }
        // Rare path
        result.push(255);
        return result;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmp_defaultcmp_shortest_sep() {
        assert_eq!(
            DefaultCmp.find_shortest_sep("abcd".as_bytes(), "abcf".as_bytes()),
            "abce".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("abc".as_bytes(), "acd".as_bytes()),
            "abc\0".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("abcdefghi".as_bytes(), "abcffghi".as_bytes()),
            "abce".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("a".as_bytes(), "a".as_bytes()),
            "a".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("a".as_bytes(), "b".as_bytes()),
            "a\0".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("abc".as_bytes(), "zzz".as_bytes()),
            "b".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("yyy".as_bytes(), "z".as_bytes()),
            "yyy\0".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_shortest_sep("".as_bytes(), "".as_bytes()),
            "".as_bytes()
        );
    }

    #[test]
    fn test_cmp_defaultcmp_short_succ() {
        assert_eq!(
            DefaultCmp.find_short_succ("abcd".as_bytes()),
            "b".as_bytes()
        );
        assert_eq!(
            DefaultCmp.find_short_succ("zzzz".as_bytes()),
            "{".as_bytes()
        );
        assert_eq!(DefaultCmp.find_short_succ(&[]), &[0xff]);
        assert_eq!(
            DefaultCmp.find_short_succ(&[0xff, 0xff, 0xff]),
            &[0xff, 0xff, 0xff, 0xff]
        );
    }
}
