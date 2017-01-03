use std::cmp::Ordering;

/// Comparator trait, supporting types that can be nested (i.e., add additional functionality on
/// top of an inner comparator)
pub trait Cmp {
    fn cmp(&self, &[u8], &[u8]) -> Ordering;
    fn find_shortest_sep(&self, &[u8], &[u8]) -> Vec<u8>;
    fn find_short_succ(&self, &[u8]) -> Vec<u8>;
}

/// Lexical comparator.
#[derive(Clone)]
pub struct DefaultCmp;

impl Cmp for DefaultCmp {
    fn cmp(&self, a: &[u8], b: &[u8]) -> Ordering {
        a.cmp(b)
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
        return a.to_vec();
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
        assert_eq!(DefaultCmp.find_shortest_sep("abcd".as_bytes(), "abcf".as_bytes()),
                   "abce".as_bytes());
        assert_eq!(DefaultCmp.find_shortest_sep("abc".as_bytes(), "acd".as_bytes()),
                   "abc".as_bytes());
        assert_eq!(DefaultCmp.find_shortest_sep("abcdefghi".as_bytes(), "abcffghi".as_bytes()),
                   "abce".as_bytes());
        assert_eq!(DefaultCmp.find_shortest_sep("a".as_bytes(), "a".as_bytes()),
                   "a".as_bytes());
        assert_eq!(DefaultCmp.find_shortest_sep("a".as_bytes(), "b".as_bytes()),
                   "a".as_bytes());
        assert_eq!(DefaultCmp.find_shortest_sep("abc".as_bytes(), "zzz".as_bytes()),
                   "b".as_bytes());
        assert_eq!(DefaultCmp.find_shortest_sep("".as_bytes(), "".as_bytes()),
                   "".as_bytes());
    }

    #[test]
    fn test_cmp_defaultcmp_short_succ() {
        assert_eq!(DefaultCmp.find_short_succ("abcd".as_bytes()),
                   "b".as_bytes());
        assert_eq!(DefaultCmp.find_short_succ("zzzz".as_bytes()),
                   "{".as_bytes());
        assert_eq!(DefaultCmp.find_short_succ(&[]), &[0xff]);
        assert_eq!(DefaultCmp.find_short_succ(&[0xff, 0xff, 0xff]),
                   &[0xff, 0xff, 0xff, 0xff]);
    }
}
