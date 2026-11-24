//! The jumplist: where the user was before each jump, walked with `Ctrl-o` and `Ctrl-i`. No I/O.

use crate::app::Request;

/// How many spots the list keeps.
const MAX_SPOTS: usize = 100;

/// A view of a function with the cursor in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub request: Request,
    /// The function's name, for the title.
    pub name: String,
    pub line: usize,
    pub col: usize,
}

/// Spots in the order they were left, and which one the user walked back to.
// ponytail: a stack like vim's `jumpoptions=stack`: a jump after `Ctrl-o` forgets the spots ahead.
#[derive(Default)]
pub struct Jumps {
    spots: Vec<Spot>,
    /// The spot walked back to; the length of `spots` while the user is at the end of the walk.
    at: usize,
}

impl Jumps {
    /// Remembers `spot`, the place a jump leaves; the spots ahead of the one walked back to are forgotten.
    pub fn record(&mut self, spot: Spot) {
        self.spots.truncate(self.at);
        if self.spots.last() != Some(&spot) {
            self.spots.push(spot);
        }
        if self.spots.len() > MAX_SPOTS {
            self.spots.remove(0);
        }
        self.at = self.spots.len();
    }

    /// `Ctrl-o`: the spot before the one walked back to; at the end of the walk, `here` is remembered first.
    pub fn back(&mut self, here: Spot) -> Option<Spot> {
        if self.at == self.spots.len() {
            self.record(here);
            self.at = self.spots.len() - 1;
        }
        self.at = self.at.checked_sub(1)?;
        self.spots.get(self.at).cloned()
    }

    /// `Ctrl-i`: the spot after the one walked back to, if any.
    pub fn forward(&mut self) -> Option<Spot> {
        let spot = self.spots.get(self.at + 1)?.clone();
        self.at += 1;
        Some(spot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::View;

    fn spot(line: usize) -> Spot {
        let request = Request {
            view: View::Decompile,
            address: 0x10,
        };
        Spot {
            request,
            name: "f".to_string(),
            line,
            col: 0,
        }
    }

    fn lines(spots: impl IntoIterator<Item = Option<Spot>>) -> Vec<Option<usize>> {
        spots.into_iter().map(|s| s.map(|s| s.line)).collect()
    }

    #[test]
    fn back_and_forward_walk_the_spots_and_end_at_the_ends() {
        let mut jumps = Jumps::default();
        assert_eq!(jumps.back(spot(9)), None);
        jumps.record(spot(1));
        jumps.record(spot(2));
        let walk = [
            jumps.back(spot(3)),
            jumps.back(spot(3)),
            jumps.back(spot(3)),
            jumps.forward(),
            jumps.forward(),
            jumps.forward(),
        ];
        let want = [Some(2), Some(1), None, Some(2), Some(3), None];
        assert_eq!(lines(walk), want);
    }

    #[test]
    fn the_same_spot_is_not_remembered_twice_in_a_row() {
        let mut jumps = Jumps::default();
        jumps.record(spot(1));
        jumps.record(spot(1));
        jumps.record(spot(2));
        assert_eq!(jumps.spots.len(), 2);
        assert_eq!(jumps.back(spot(2)).map(|s| s.line), Some(1));
    }

    #[test]
    fn a_jump_after_walking_back_forgets_the_way_forward() {
        let mut jumps = Jumps::default();
        jumps.record(spot(1));
        jumps.record(spot(2));
        assert_eq!(jumps.back(spot(3)).map(|s| s.line), Some(2));
        jumps.record(spot(2));
        assert_eq!(jumps.forward(), None);
        assert_eq!(jumps.back(spot(4)).map(|s| s.line), Some(2));
        assert_eq!(jumps.back(spot(4)).map(|s| s.line), Some(1));
    }

    #[test]
    fn only_the_newest_spots_are_kept() {
        let mut jumps = Jumps::default();
        for line in 0..MAX_SPOTS + 50 {
            jumps.record(spot(line));
        }
        assert_eq!(jumps.spots.len(), MAX_SPOTS);
        assert_eq!(jumps.spots[0].line, 50);
        assert_eq!(jumps.back(spot(999)).map(|s| s.line), Some(MAX_SPOTS + 49));
    }
}
