<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Selection and the selection filter

Click (MB1) a body in the graphics window, or a feature in the Part
Navigator. The selection is drawn in orange. Esc or a click on empty space
clears it.

The **selection filter** on the top border bar decides what a click picks:

| Filter | Picks |
| --- | --- |
| No Selection Filter | the face under the cursor |
| Solid Body | the whole body |
| Face | the face under the cursor |
| Feature | the body of the feature, and its Part Navigator row |

Picking uses the GPU: it reads which face is drawn under the cursor, so
hidden faces are never picked. Edge picking, rectangle selection and
preselection highlighting arrive in Stage 1.
