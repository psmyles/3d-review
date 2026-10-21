//! Union-find over `u32` ids: the one grouping structure the rebuild and the
//! wrap use wherever they ask "which of these are connected".
//!
//! Four passes need it - a vertex's fans and the mesh's components
//! ([`super::topology`]), the source's attribute regions ([`super::project`])
//! and a wrapped shell's pieces (`crate::shrinkwrap::mc`) - and each had grown
//! a copy, with three different rules for which root wins a merge.
//!
//! ## The lower root wins
//!
//! So a set's root is always its **smallest member**, whatever order the pairs
//! were joined in. That is the property every caller wants and none of the
//! copies had all of: an id read off a root is then a function of the sets
//! rather than of the walk, which is what lets a component be numbered, a tie
//! between two shells be broken, or two runs be compared, without a second
//! pass to canonicalize the answer. Path halving alone keeps the trees shallow
//! enough; union by size would buy little here and would give up the rule.

/// Disjoint sets over `0..count`.
#[derive(Debug, Default)]
pub(crate) struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    /// Every id in a set of its own.
    pub(crate) fn new(count: usize) -> Self {
        let mut sets = Self::default();
        sets.reset(count);
        sets
    }

    /// Start over with `count` singleton sets, keeping the allocation: the fan
    /// test runs one of these per vertex, millions of times.
    pub(crate) fn reset(&mut self, count: usize) {
        self.parent.clear();
        self.parent.extend(0..count as u32);
    }

    /// The root of `node`'s set, which is its smallest member.
    pub(crate) fn find(&mut self, mut node: u32) -> u32 {
        while self.parent[node as usize] != node {
            // Path halving: point each node at its grandparent as we climb.
            self.parent[node as usize] = self.parent[self.parent[node as usize] as usize];
            node = self.parent[node as usize];
        }
        node
    }

    /// Join the sets holding `a` and `b`.
    pub(crate) fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            let (low, high) = if a < b { (a, b) } else { (b, a) };
            self.parent[high as usize] = low;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_is_rooted_at_its_smallest_member_whatever_the_order() {
        let pairs = [(5u32, 3u32), (3, 9), (7, 2), (9, 7), (0, 1)];
        let mut forward = UnionFind::new(10);
        for &(a, b) in &pairs {
            forward.union(a, b);
        }
        let mut backward = UnionFind::new(10);
        for &(a, b) in pairs.iter().rev() {
            backward.union(b, a);
        }

        for node in 0..10 {
            assert_eq!(forward.find(node), backward.find(node));
        }
        assert_eq!(forward.find(9), 2, "{{2, 3, 5, 7, 9}} is rooted at 2");
        assert_eq!(forward.find(1), 0);
        assert_eq!(forward.find(4), 4, "an id never joined is its own set");
    }

    #[test]
    fn a_reset_starts_over() {
        let mut sets = UnionFind::new(4);
        sets.union(0, 3);
        sets.reset(2);
        assert_eq!(sets.find(1), 1);
        assert_eq!(sets.find(0), 0);
    }
}
