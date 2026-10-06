//! The People pane: shown in place of the folder sidebar while the People
//! view is on, it lists everyone the user exchanges mail with, most recent
//! first, the way a messenger lists its chats. Picking a person fills the
//! message list with that conversation, whichever folder and account its
//! mail sits in; All People at the top shows every exchange.
//!
//! The list is refreshed after every sync, so rows are kept per address and
//! updated in place, and reordered only when the order changed. Rows carry
//! no handlers of their own: activation, filtering and the context menu are
//! the list's, so a row that goes away is freed with it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use relm4::prelude::*;

use crate::i18n::i18n;
use crate::people::Person;
use crate::ui::context_menu::{show_context_menu, MenuEntry};
use crate::ui::sidebar::{pin_icon_size, style_badge};

/// One person's row and the parts updated in place.
struct Row {
    row: gtk::ListBoxRow,
    avatar: adw::Avatar,
    name: gtk::Label,
    badge: gtk::Label,
    unread: u32,
}

pub struct PeoplePane {
    list: gtk::ListBox,
    all_row: gtk::ListBoxRow,
    all_badge: gtk::Label,
    /// Everyone's unread mail, for the All People badge.
    all_unread: u32,
    rows: HashMap<String, Row>,
    /// The addresses in list order (after All People).
    order: Vec<String>,
    /// Lower-case "name address" per row, for the filter.
    haystack: Rc<RefCell<HashMap<gtk::ListBoxRow, String>>>,
    /// The person shown (`None`: All People).
    selected: Option<String>,
    /// Set while the pane moves its own highlight, so that is not taken
    /// for a click.
    quiet: Rc<Cell<bool>>,
}

#[derive(Debug, Clone)]
pub enum PeoplePaneInput {
    /// The People list, most recent first.
    SetPeople(Vec<Person>),
    /// One person's unread count changed (mail read or marked unread).
    SetUnread { address: String, unread: u32 },
    /// Highlight a person (`None`: All People) without reporting it.
    Select(Option<String>),
    /// A row was picked (its index in the list).
    Picked(i32),
    /// Right-click at a point of the list.
    Menu { x: f64, y: f64 },
}

#[derive(Debug)]
pub enum PeoplePaneOutput {
    /// A person was picked (`None`: All People).
    Selected(Option<String>),
    /// Write to this address.
    Compose(String),
}

impl SimpleComponent for PeoplePane {
    type Init = ();
    type Input = PeoplePaneInput;
    type Output = PeoplePaneOutput;
    type Root = gtk::Box;
    type Widgets = ();

    fn init_root() -> Self::Root {
        gtk::Box::new(gtk::Orientation::Vertical, 0)
    }

    fn init(_: (), root: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        root.add_css_class("people-pane");
        let filter = gtk::SearchEntry::new();
        filter.set_placeholder_text(Some(i18n("Filter People").as_str()));
        filter.set_margin_start(8);
        filter.set_margin_end(8);
        filter.set_margin_top(4);
        filter.set_margin_bottom(4);
        root.append(&filter);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.add_css_class("navigation-sidebar");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_vexpand(true);
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_child(Some(&list));
        root.append(&scroller);

        // All People, above everyone.
        let all_row = gtk::ListBoxRow::new();
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        hbox.add_css_class("folder-row");
        let icon = gtk::Image::from_icon_name("system-users-symbolic");
        pin_icon_size(&icon);
        icon.set_size_request(28, -1);
        icon.add_css_class("folder-icon");
        hbox.append(&icon);
        let label = gtk::Label::new(Some(i18n("All People").as_str()));
        label.set_hexpand(true);
        label.set_halign(gtk::Align::Start);
        label.add_css_class("account-name");
        hbox.append(&label);
        let all_badge = badge();
        hbox.append(&all_badge);
        all_row.set_child(Some(&hbox));
        list.append(&all_row);

        let haystack: Rc<RefCell<HashMap<gtk::ListBoxRow, String>>> = Rc::default();
        {
            let haystack = haystack.clone();
            let filter = filter.downgrade();
            list.set_filter_func(move |row| {
                let Some(filter) = filter.upgrade() else { return true };
                let needle = filter.text().trim().to_lowercase();
                needle.is_empty()
                    || haystack.borrow().get(row).is_none_or(|h| h.contains(&needle))
            });
        }
        {
            let list = list.downgrade();
            filter.connect_search_changed(move |_| {
                if let Some(list) = list.upgrade() {
                    list.invalidate_filter();
                }
            });
        }

        let quiet = Rc::new(Cell::new(false));
        {
            let quiet = quiet.clone();
            let s = sender.input_sender().clone();
            list.connect_row_selected(move |_, row| {
                if quiet.get() {
                    return;
                }
                if let Some(row) = row {
                    let _ = s.send(PeoplePaneInput::Picked(row.index()));
                }
            });
        }
        let right_click = gtk::GestureClick::new();
        right_click.set_button(3);
        {
            let s = sender.input_sender().clone();
            right_click.connect_pressed(move |_, _, x, y| {
                let _ = s.send(PeoplePaneInput::Menu { x, y });
            });
        }
        list.add_controller(right_click);

        let model = PeoplePane {
            list,
            all_row,
            all_badge,
            all_unread: 0,
            rows: HashMap::new(),
            order: Vec::new(),
            haystack,
            selected: None,
            quiet,
        };
        model.highlight();
        ComponentParts { model, widgets: () }
    }

    fn update(&mut self, msg: PeoplePaneInput, sender: ComponentSender<Self>) {
        match msg {
            PeoplePaneInput::SetPeople(people) => self.set_people(people),
            PeoplePaneInput::SetUnread { address, unread } => {
                let Some(r) = self.rows.get_mut(&address) else { return };
                self.all_unread = self.all_unread - r.unread + unread;
                r.unread = unread;
                set_count(&r.badge, unread);
                set_count(&self.all_badge, self.all_unread);
            }
            PeoplePaneInput::Select(address) => {
                self.selected = address;
                self.highlight();
            }
            PeoplePaneInput::Picked(index) => {
                let Some(address) = self.address_at(index) else { return };
                let address = address.cloned();
                if address != self.selected {
                    self.selected = address.clone();
                    let _ = sender.output(PeoplePaneOutput::Selected(address));
                }
            }
            PeoplePaneInput::Menu { x, y } => {
                let Some(row) = self.list.row_at_y(y as i32) else { return };
                let Some(Some(address)) = self.address_at(row.index()).map(|a| a.cloned()) else {
                    return;
                };
                let s = sender.output_sender().clone();
                let to = address.clone();
                let list = self.list.clone();
                show_context_menu(
                    &self.list,
                    x,
                    y,
                    vec![vec![
                        MenuEntry::new(i18n("Write To"), move || {
                            let _ = s.send(PeoplePaneOutput::Compose(to.clone()));
                        })
                        .icon("mail-message-new-symbolic"),
                        MenuEntry::new(i18n("Copy Address"), move || {
                            list.clipboard().set_text(&address);
                        })
                        .icon("edit-copy-symbolic"),
                    ]],
                );
            }
        }
    }
}

impl PeoplePane {
    fn set_people(&mut self, people: Vec<Person>) {
        self.all_unread = 0;
        for p in &people {
            self.all_unread += p.unread;
            let r = self.rows.entry(p.address.clone()).or_insert_with(|| person_row(&p.address));
            let name = p.display_name();
            if r.name.label() != name {
                r.name.set_label(name);
                r.avatar.set_text(Some(name));
                self.haystack
                    .borrow_mut()
                    .insert(r.row.clone(), format!("{} {}", p.name.to_lowercase(), p.address));
            }
            r.unread = p.unread;
            set_count(&r.badge, p.unread);
        }
        set_count(&self.all_badge, self.all_unread);
        let same_order = people.len() == self.order.len()
            && people.iter().zip(&self.order).all(|(p, a)| p.address == *a);
        if same_order {
            return;
        }
        self.quiet.set(true);
        for address in &self.order {
            if let Some(r) = self.rows.get(address) {
                self.list.remove(&r.row);
            }
        }
        self.order = people.into_iter().map(|p| p.address).collect();
        let keep: std::collections::HashSet<&String> = self.order.iter().collect();
        let gone: Vec<String> = self.rows.keys().filter(|a| !keep.contains(a)).cloned().collect();
        for address in gone {
            if let Some(r) = self.rows.remove(&address) {
                self.haystack.borrow_mut().remove(&r.row);
            }
        }
        for address in &self.order {
            self.list.append(&self.rows[address].row);
        }
        self.quiet.set(false);
        self.highlight();
    }

    /// The person a list row stands for: `Some(None)` for All People (row
    /// 0), `None` past the end.
    fn address_at(&self, index: i32) -> Option<Option<&String>> {
        match index {
            0 => Some(None),
            i if i > 0 => self.order.get(i as usize - 1).map(Some),
            _ => None,
        }
    }

    /// Move the highlight to the shown person, quietly.
    fn highlight(&self) {
        let row = match &self.selected {
            None => Some(&self.all_row),
            Some(a) => self.rows.get(a).map(|r| &r.row),
        };
        self.quiet.set(true);
        match row {
            Some(row) => self.list.select_row(Some(row)),
            None => self.list.unselect_all(),
        }
        self.quiet.set(false);
    }
}

fn badge() -> gtk::Label {
    let badge = gtk::Label::new(None);
    style_badge(&badge, 5);
    badge.set_visible(false);
    badge
}

fn set_count(badge: &gtk::Label, n: u32) {
    badge.set_visible(n > 0);
    let text = n.to_string();
    if badge.label() != text {
        badge.set_label(&text);
    }
}

fn person_row(address: &str) -> Row {
    let row = gtk::ListBoxRow::new();
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    hbox.add_css_class("folder-row");
    let avatar = adw::Avatar::new(28, Some(address), true);
    hbox.append(&avatar);
    let name = gtk::Label::new(None);
    name.set_hexpand(true);
    name.set_halign(gtk::Align::Start);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.add_css_class("account-name");
    hbox.append(&name);
    let badge = badge();
    hbox.append(&badge);
    row.set_child(Some(&hbox));
    row.set_tooltip_text(Some(address));
    Row { row, avatar, name, badge, unread: 0 }
}
