//! CSI parameter scanning without allocation.

/// Marks a parameter that was present but empty (`CSI ;5u`).
pub(crate) const EMPTY: u32 = u32::MAX;
const MAX_VALS: usize = 40;
const MAX_GROUPS: usize = 16;

/// A scanned CSI sequence. Parameters are `;`-separated groups of
/// `:`-separated values.
#[derive(Clone, Copy)]
pub(crate) struct Csi {
    /// Private-parameter prefix (`<` `=` `>` `?`) or 0.
    pub prefix: u8,
    /// Last intermediate byte or 0.
    pub inter: u8,
    pub fin: u8,
    /// Something did not fit the shapes we interpret (too many parameters,
    /// several intermediates, prefix bytes in the middle).
    pub odd: bool,
    vals: [u32; MAX_VALS],
    nvals: u8,
    /// Index into `vals` of each group's first value.
    groups: [u8; MAX_GROUPS + 1],
    ngroups: u8,
    /// No parameter bytes at all.
    pub bare: bool,
}

pub(crate) enum Scan {
    /// Bytes consumed after the introducer, including the final byte.
    Done(Csi, usize),
    Incomplete,
    /// A byte that cannot appear in a CSI sequence at this offset.
    Invalid(usize),
}

/// Longest CSI body accepted; anything longer is treated as garbage.
pub(crate) const MAX_CSI: usize = 128;

pub(crate) fn scan(b: &[u8]) -> Scan {
    let mut c = Csi {
        prefix: 0,
        inter: 0,
        fin: 0,
        odd: false,
        vals: [EMPTY; MAX_VALS],
        nvals: 1,
        groups: [0; MAX_GROUPS + 1],
        ngroups: 1,
        bare: true,
    };
    let mut i = 0;
    if let Some(&p) = b.first()
        && (0x3c..=0x3f).contains(&p)
    {
        c.prefix = p;
        i = 1;
    }
    while i < b.len() {
        if i >= MAX_CSI {
            return Scan::Invalid(i);
        }
        let x = b[i];
        match x {
            b'0'..=b'9' => {
                if c.inter != 0 {
                    c.odd = true;
                }
                c.bare = false;
                let slot = &mut c.vals[c.nvals as usize - 1];
                let cur = if *slot == EMPTY { 0 } else { *slot };
                *slot = cur.saturating_mul(10).saturating_add((x - b'0') as u32).min(EMPTY - 1);
            }
            b':' | b';' => {
                if c.inter != 0 {
                    c.odd = true;
                }
                c.bare = false;
                if (c.nvals as usize) < MAX_VALS {
                    c.nvals += 1;
                } else {
                    c.odd = true;
                    i += 1;
                    continue;
                }
                if x == b';' {
                    if (c.ngroups as usize) < MAX_GROUPS {
                        c.groups[c.ngroups as usize] = c.nvals - 1;
                        c.ngroups += 1;
                    } else {
                        c.odd = true;
                    }
                }
            }
            0x3c..=0x3f => c.odd = true,
            0x20..=0x2f => {
                if c.inter != 0 {
                    c.odd = true;
                }
                c.inter = x;
            }
            0x40..=0x7e => {
                c.fin = x;
                c.groups[c.ngroups as usize] = c.nvals;
                return Scan::Done(c, i + 1);
            }
            _ => return Scan::Invalid(i),
        }
        i += 1;
    }
    Scan::Incomplete
}

impl Csi {
    pub fn nparams(&self) -> usize {
        if self.bare { 0 } else { self.ngroups as usize }
    }

    /// Values of group `g` (empty slice if absent).
    pub fn group(&self, g: usize) -> &[u32] {
        if g >= self.nparams() {
            return &[];
        }
        let s = self.groups[g] as usize;
        let e = self.groups[g + 1] as usize;
        &self.vals[s..e]
    }

    /// Sub-value `s` of group `g`, or `default` when absent or empty.
    pub fn sub(&self, g: usize, s: usize, default: u32) -> u32 {
        match self.group(g).get(s) {
            Some(&v) if v != EMPTY => v,
            _ => default,
        }
    }

    pub fn param(&self, g: usize, default: u32) -> u32 {
        self.sub(g, 0, default)
    }

    /// True if any group has more than one value.
    pub fn has_subparams(&self) -> bool {
        (0..self.nparams()).any(|g| self.group(g).len() > 1)
    }
}
