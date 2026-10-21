use target::format_amount;
#[test] fn rounds_half_away() { assert_eq!(format_amount(1_234_567), "1,234.57"); assert_eq!(format_amount(5), "0.01"); assert_eq!(format_amount(-5), "-0.01"); assert_eq!(format_amount(4), "0.00"); }
#[test] fn rounding_carries_into_groups() { assert_eq!(format_amount(999_995), "1,000.00"); }
#[test] fn no_negative_zero() { assert_eq!(format_amount(-4), "0.00"); assert_eq!(format_amount(0), "0.00"); }
#[test] fn groups() { assert_eq!(format_amount(1_000_000_000), "1,000,000.00"); assert_eq!(format_amount(-12_345_000), "-12,345.00"); }
#[test] fn extremes() { assert_eq!(format_amount(i64::MIN), "-9,223,372,036,854,775.81"); assert_eq!(format_amount(i64::MAX), "9,223,372,036,854,775.81"); }
