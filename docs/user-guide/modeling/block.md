<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Block

**Home › Feature › Block.** Creates a rectangular block.

| Group | Field | Meaning |
| --- | --- | --- |
| Origin | Origin XC, YC, ZC | The block's corner, along the work coordinate system's axes |
| Dimensions | Length (XC), Width (YC), Height (ZC) | Edge lengths |
| | Name | Optional. Empty gives `Block(n)`, numbered by history position |

Every field takes a formula: `25`, `25 mm`, `1 in`, `p0 * 2`. A bare
number is in the part's unit. Each value becomes an expression (`p0`,
`p1`, …), as in NX.

Press **OK**, MB2 or Enter to create the block and close the dialog.
**Apply** creates it and keeps the dialog open for another. **Cancel** or
Esc closes it without creating anything.

While the dialog is open, the field you last edited follows the cursor in
the graphics window, so you can type without leaving the model. A value
the document refuses, such as a bad formula or a length given as an angle,
keeps the dialog open with the reason in red, and nothing is created.

The block is computed by the geometry kernel. If the kernel refuses it,
the feature is still created, marked red in the Part Navigator, and the
reason is shown on hover and in Information.
