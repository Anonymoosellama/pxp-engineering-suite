// SPDX-License-Identifier: LGPL-2.1-or-later
//! Property tests: unit conversions round-trip and dimensions combine
//! consistently for arbitrary inputs.

use proptest::prelude::*;
use pxp_units::{Dim, Length, Quantity, UnitSystem, parse_unit};

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

proptest! {
    #[test]
    fn inch_round_trip(v in -1.0e9_f64..1.0e9) {
        prop_assert!(close(Length::from_inch(v).inch(), v));
        prop_assert!(close(Length::from_mm(v).mm(), v));
    }

    #[test]
    fn every_display_unit_round_trips(v in -1.0e6_f64..1.0e6, which in 0usize..14) {
        let dims = [Dim::LENGTH, Dim::AREA, Dim::VOLUME, Dim::MASS, Dim::TIME, Dim::ANGLE,
            Dim::FREQUENCY, Dim::VELOCITY, Dim::ACCELERATION, Dim::FORCE, Dim::STRESS,
            Dim::ENERGY, Dim::DENSITY, Dim::LENGTH];
        for system in [UnitSystem::Millimetre, UnitSystem::Inch] {
            if let Some(unit) = system.display_unit(dims[which]).unwrap() {
                let q = unit.quantity(v);
                prop_assert!(close(unit.value_of(q).unwrap(), v));
            }
        }
    }

    #[test]
    fn product_then_quotient_restores_dimension(a in 1.0e-3_f64..1.0e3, b in 1.0e-3_f64..1.0e3) {
        let force = parse_unit("N").unwrap().quantity(a);
        let area = parse_unit("mm^2").unwrap().quantity(b);
        let stress = force.checked_div(area).unwrap();
        prop_assert_eq!(stress.dim(), Dim::STRESS);
        let back = stress.checked_mul(area).unwrap();
        prop_assert_eq!(back.dim(), Dim::FORCE);
        prop_assert!(close(back.value(), a));
    }

    #[test]
    fn addition_never_mixes_dimensions(a in -1.0e3_f64..1.0e3, b in -1.0e3_f64..1.0e3) {
        let l = Quantity::from_internal(a, Dim::LENGTH);
        let t = Quantity::from_internal(b, Dim::ANGLE);
        prop_assert!(l.checked_add(t).is_err());
        prop_assert!(l.checked_add(l).is_ok());
    }
}
