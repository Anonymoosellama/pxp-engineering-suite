<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Starting PXP and the window layout

Run `pxp`, or `pxp part.pxp` to open a part straight away.

| Area | Where | What it does |
| --- | --- | --- |
| Ribbon | top | **File** menu, the active application's tabs, and command buttons grouped as in NX. Greyed buttons are planned; hover for "Planned for Stage N." |
| Top border bar | under the ribbon | Selection filter, Undo and Redo, the View menu, Dark theme, and the Command Finder (right) |
| Resource bar | left | Part Navigator, Assembly Navigator (Stage 1C) and Roles. Drag the tabs to re-dock or float them |
| Graphics window | centre | The model, the view triad (bottom-left), the WCS (XC/YC/ZC) and datum axes |
| Cue line | bottom | What to do next |
| Status line | bottom | What just happened. Warnings are amber and errors red |

The graphics window's tab shows the part's file name, with **(Modified)**
when there are unsaved changes. Without a usable GPU, PXP draws with the
operating system's software rasteriser and says so on the status line.
Large models are then slow.

**Help** (top right) shows the version, the geometry kernel, the graphics
adapter and where crash reports are kept.
