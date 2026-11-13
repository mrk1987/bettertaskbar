//! System menus live outside the widget's tiny client area.
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::*;

pub enum Item<T> {
    Command(String, bool, T),
    Separator,
    Submenu(String, Vec<Item<T>>),
}

struct Menu(HMENU);
impl Drop for Menu {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.0);
        }
    }
}

struct Owner(HWND);
impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

fn wide(label: &str) -> Vec<u16> {
    // Place names are text, not menu accelerators.
    label
        .replace('&', "&&")
        .replace('\0', "")
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

fn build<T>(items: Vec<Item<T>>, actions: &mut Vec<T>) -> windows::core::Result<Menu> {
    let menu = Menu(unsafe { CreatePopupMenu()? });
    for item in items {
        unsafe {
            match item {
                Item::Separator => AppendMenuW(menu.0, MF_SEPARATOR, 0, PCWSTR::null())?,
                Item::Command(label, checked, action) => {
                    actions.push(action);
                    let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
                    AppendMenuW(menu.0, flags, actions.len(), PCWSTR(wide(&label).as_ptr()))?;
                }
                Item::Submenu(label, items) => {
                    let child = build(items, actions)?;
                    AppendMenuW(
                        menu.0,
                        MF_POPUP,
                        child.0 .0 as usize,
                        PCWSTR(wide(&label).as_ptr()),
                    )?;
                    // The parent menu owns its submenus after AppendMenu succeeds.
                    std::mem::forget(child);
                }
            }
        }
    }
    Ok(menu)
}

pub fn show<T>(items: Vec<Item<T>>) -> windows::core::Result<Option<T>> {
    let mut actions = Vec::new();
    let menu = build(items, &mut actions)?;
    let pt = crate::shell::cursor();
    unsafe {
        // TrackPopupMenu needs a top-level owner. A docked widget is a child
        // of Explorer, so use our own zero-size tool window, never Explorer.
        let owner = Owner(CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("STATIC"),
            w!("BetterTaskbar Menu"),
            WS_POPUP,
            pt.x,
            pt.y,
            0,
            0,
            None,
            None,
            None,
            None,
        )?);
        let _ = ShowWindow(owner.0, SW_SHOW);
        let _ = SetForegroundWindow(owner.0);
        let id = TrackPopupMenu(
            menu.0,
            TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
            pt.x,
            pt.y,
            None,
            owner.0,
            None,
        )
        .0 as usize;
        Ok(id
            .checked_sub(1)
            .and_then(|index| (index < actions.len()).then(|| actions.swap_remove(index))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submenu_commands_map_to_their_actions_and_labels_are_literal() {
        let mut actions = Vec::new();
        let menu = build(
            vec![
                Item::Command("Dock".into(), true, 10),
                Item::Separator,
                Item::Submenu(
                    "Place".into(),
                    vec![Item::Command("A & B".into(), false, 20)],
                ),
            ],
            &mut actions,
        )
        .unwrap();
        unsafe {
            assert_eq!(GetMenuItemCount(Some(menu.0)), 3);
            assert_eq!(GetMenuItemID(menu.0, 0), 1);
            let submenu = GetSubMenu(menu.0, 2);
            assert_eq!(GetMenuItemID(submenu, 0), 2);
        }
        assert_eq!(actions, [10, 20]);
        assert_eq!(String::from_utf16_lossy(&wide("A & B\0")), "A && B\0");
    }
}
