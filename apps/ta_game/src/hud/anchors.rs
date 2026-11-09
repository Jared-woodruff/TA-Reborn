//! Where the interface drew, this frame, what a lesson may point at: the build menu's
//! pictures, tabs and pages, the order buttons, the MAP menu and a constructor's queue. The
//! modern deck and TA's classic panel each fill their own; the tutorial's guide reads them
//! to point at the next thing to click.

use egui::Rect;

#[derive(Default)]
pub struct Anchors {
    /// TA's classic interface drew them (its build pages, no tabs or MAP menu).
    pub classic: bool,
    /// The build menu's pictures in view, by unit type.
    build: Vec<(usize, Rect)>,
    /// The deck's build tabs not shown: where each is, and what it holds.
    tabs: Vec<(Rect, Vec<usize>)>,
    /// The build menu's options (of the tab shown), each with its page; the page shown.
    pages: Vec<(usize, usize)>,
    page: usize,
    /// The buttons to the page before and after (where they can be pressed).
    prev_page: Option<Rect>,
    next_page: Option<Rect>,
    /// The classic panel's BUILD button, while it shows the orders.
    build_page: Option<Rect>,
    /// Order buttons, by their key binding's action ("fight", "reclaim").
    orders: Vec<(&'static str, Rect)>,
    /// The top bar's MAP menu.
    pub map_menu: Option<Rect>,
    /// The first of the selected constructor's queued buildings a click moves up.
    pub queue: Option<Rect>,
    /// TA's side panel (the classic interface), down the screen's left edge.
    pub side_panel: Option<Rect>,
    /// A factory's Repeat switch.
    pub repeat: Option<Rect>,
}

impl Anchors {
    /// A new frame: nothing drawn yet.
    pub fn clear(&mut self, classic: bool) {
        *self = Anchors { classic, ..Anchors::default() };
    }

    pub fn picture(&mut self, type_id: usize, rect: Rect) {
        self.build.push((type_id, rect));
    }

    /// A build tab not shown, holding `options`.
    pub fn tab(&mut self, rect: Rect, options: Vec<usize>) {
        self.tabs.push((rect, options));
    }

    /// The build menu's options with their pages, and the page shown.
    pub fn pages(&mut self, options: impl IntoIterator<Item = (usize, usize)>, page: usize) {
        self.pages.extend(options);
        self.page = page;
    }

    /// The pager's buttons that can be pressed.
    pub fn pager(&mut self, prev: Option<Rect>, next: Option<Rect>) {
        self.prev_page = prev;
        self.next_page = next;
    }

    pub fn build_page(&mut self, rect: Rect) {
        self.build_page = Some(rect);
    }

    pub fn order(&mut self, action: &'static str, rect: Rect) {
        self.orders.push((action, rect));
    }

    /// The order button of `action`, if drawn.
    pub fn order_button(&self, action: &str) -> Option<Rect> {
        self.orders.iter().find(|(a, _)| *a == action).map(|(_, r)| *r)
    }

    /// The next click towards building `type_id` with the selected builder: its picture, else
    /// the tab holding it, else the pager towards its page, else the classic panel's BUILD
    /// button.
    pub fn towards_build(&self, type_id: usize) -> Option<Rect> {
        if let Some(&(_, r)) = self.build.iter().find(|(t, _)| *t == type_id) {
            return Some(r);
        }
        if let Some((r, _)) = self.tabs.iter().find(|(_, holds)| holds.contains(&type_id)) {
            return Some(*r);
        }
        if let Some(&(_, p)) = self.pages.iter().find(|(t, _)| *t == type_id) {
            let (towards, back) = if p > self.page { (self.next_page, self.prev_page) } else { (self.prev_page, self.next_page) };
            // (TA's pages go round: the other way gets there too.)
            return towards.or(back);
        }
        self.build_page
    }
}

#[cfg(test)]
mod tests {
    use super::Anchors;
    use egui::{Rect, pos2, vec2};

    fn at(x: f32) -> Rect {
        Rect::from_min_size(pos2(x, 0.0), vec2(10.0, 10.0))
    }

    /// The guide is led to a picture through its tab or the pages, whichever hides it.
    #[test]
    fn the_way_to_a_picture() {
        let mut a = Anchors::default();
        a.clear(false);
        a.pages([(1, 1), (2, 2), (3, 0), (4, 1)], 1);
        a.picture(1, at(10.0));
        a.picture(4, at(11.0));
        a.tab(at(20.0), vec![7]);
        a.pager(Some(at(30.0)), Some(at(31.0)));
        assert_eq!(a.towards_build(1), Some(at(10.0)), "in view: the picture itself");
        assert_eq!(a.towards_build(7), Some(at(20.0)), "on another tab: the tab");
        assert_eq!(a.towards_build(2), Some(at(31.0)), "on a later page: the next page");
        assert_eq!(a.towards_build(3), Some(at(30.0)), "on an earlier page: the page before");
        assert_eq!(a.towards_build(9), None, "not in this builder's menu");
        // On the last page with no way on: back.
        a.pager(Some(at(30.0)), None);
        assert_eq!(a.towards_build(2), Some(at(30.0)));
        // TA's panel showing the orders: its BUILD button first.
        a.clear(true);
        a.build_page(at(40.0));
        assert_eq!(a.towards_build(1), Some(at(40.0)));
        assert!(a.classic);
    }
}
