# ADR-0007: Internal unit system and expression conventions

- **Status:** Accepted (implemented in session 1, `crates/pxp-units`,
  `crates/pxp-doc/src/expr.rs`)
- **Date:** 2026-11-20

## Context

The master prompt fixes the internal length unit (millimetre) and angle unit
(radian) and requires unit newtypes. It leaves the other base units open, and
Stage 5 FEA needs one consistent unit system (it suggests mm, N, MPa, tonne,
s).

## Decisions

1. **Coherent mm-t-s internal system.** Mass is stored in **tonnes**, so
   1 N = 1 t·mm/s², 1 MPa = 1 N/mm², 1 mJ = 1 N·mm, and density is t/mm³.
   Products and quotients of internal values need no conversion factors, and
   FEA input needs no conversion at all. Users see kg, N and MPa (or lbm,
   lbf, psi) through `UnitSystem`.
2. **Angle is a tracked dimension** (unlike SI). `10mm + 5deg` is an error.
   Arc length is written `r * theta / 1rad`.
3. **Units attach to numbers, not names.** `25.4mm` or `7850[kg/m^3]`; a bare
   `in` is an expression name. No ambiguity between unit symbols and
   expression names.
4. **NX trigonometry convention.** `sin(30)` reads a bare number as degrees,
   as NX does; `sin(0.5rad)` honours the unit.
5. **Rounding functions take dimensionless input only,** because "round a
   length" would depend on the display unit.
6. **Exact definitions:** 1 in = 25.4 mm, 1 lb = 0.453 592 37 kg,
   1 lbf = 4.448 221 615 260 5 N, psi derived from those.

## Open item for dialogs

NX interprets a bare number typed into a length field in the part's display
unit. The expression engine returns a dimensionless value for `25`; the
dialog layer (0H) will apply the field's expected dimension and the part's
`UnitSystem::length_unit()`.
