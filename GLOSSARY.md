# Wraith

A Windows terminal: one background server owns the shells, and windows show and edit what it holds.

## Language

### Layout

**Workspace**:
A named group of tabs, with its own root folder and color. At most one window shows a workspace at a time.
_Avoid_: Session, project, window

**Attached**:
A workspace is attached while a window shows it; otherwise it is detached and waits to be picked up.
_Avoid_: Open, held

**Tab**:
One entry in a workspace's tab bar, holding a tree of split panes. Moving a tab moves all of its panes and their running shells.
_Avoid_: Shell tab, page

**Pane**:
One shell in a tab, with its own screen and scrollback.
_Avoid_: Split, terminal, view

**Quick terminal**:
The drop-down window toggled by a global hotkey, with a workspace of its own that normal windows never show.
_Avoid_: Quake mode, dropdown

### Actions

**Move tab**:
Taking a tab out of one workspace and putting it into another.
_Avoid_: Transfer, reparent

**Reorder**:
Changing a tab's position within its own workspace.
_Avoid_: Move (for this case)
