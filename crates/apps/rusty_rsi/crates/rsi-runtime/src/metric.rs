//! Task metrics: pure functions from a solution's output to a [`Score`].
//!
//! Each metric returns `None` for output it cannot score (wrong line count,
//! unparsable values); the grader then applies the task's floor. Every
//! metric is normalised so that higher is better and the result lies in
//! `[0, 1]`, which is what makes a grade averaged across tasks meaningful.

use rsi_core::Score;

/// The metric a task is scored with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Coefficient of determination, clamped to `[0, 1]`.
    R2,
    /// Mean over instances of `min(1, reference / tour length)`.
    TourRatio,
    /// Fraction of lines equal to the label after trimming.
    Accuracy,
}

impl Metric {
    /// Parses a manifest name: `r2`, `tour_ratio` or `accuracy`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "r2" => Some(Self::R2),
            "tour_ratio" => Some(Self::TourRatio),
            "accuracy" => Some(Self::Accuracy),
            _ => None,
        }
    }

    /// Scores `output` against `labels` (and `inputs`, which the tour
    /// metric needs for city coordinates).
    #[must_use]
    pub fn score(self, inputs: &str, labels: &str, output: &str) -> Option<Score> {
        let value = match self {
            Self::R2 => r2(labels, output)?,
            Self::TourRatio => tour_ratio(inputs, labels, output)?,
            Self::Accuracy => accuracy(labels, output)?,
        };
        Score::new(value).ok()
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn floats(text: &str) -> Option<Vec<f64>> {
    lines(text)
        .into_iter()
        .map(|line| line.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect()
}

fn r2(labels: &str, output: &str) -> Option<f64> {
    let truth = floats(labels)?;
    let predicted = floats(output)?;
    if truth.is_empty() || truth.len() != predicted.len() {
        return None;
    }
    let mean = truth.iter().sum::<f64>() / truth.len() as f64;
    let total: f64 = truth.iter().map(|y| (y - mean).powi(2)).sum();
    let residual: f64 = truth
        .iter()
        .zip(&predicted)
        .map(|(y, p)| (y - p).powi(2))
        .sum();
    if !residual.is_finite() {
        return Some(0.0);
    }
    if total == 0.0 {
        return Some(if residual == 0.0 { 1.0 } else { 0.0 });
    }
    Some((1.0 - residual / total).clamp(0.0, 1.0))
}

fn tour_ratio(inputs: &str, labels: &str, output: &str) -> Option<f64> {
    let instances = lines(inputs);
    let references = floats(labels)?;
    let tours = lines(output);
    if instances.is_empty() || instances.len() != references.len() || tours.len() != instances.len()
    {
        return None;
    }
    let mut total = 0.0;
    for ((instance, reference), tour) in instances.iter().zip(&references).zip(&tours) {
        total += instance_ratio(instance, *reference, tour)?;
    }
    Some(total / instances.len() as f64)
}

/// One instance's ratio; an invalid tour scores 0, malformed inputs are `None`.
fn instance_ratio(instance: &str, reference: f64, tour: &str) -> Option<f64> {
    let coords: Vec<f64> = instance
        .split_whitespace()
        .map(|v| v.parse::<f64>().ok())
        .collect::<Option<_>>()?;
    if coords.len() < 4 || !coords.len().is_multiple_of(2) || reference <= 0.0 {
        return None;
    }
    let cities: Vec<(f64, f64)> = coords.chunks(2).map(|p| (p[0], p[1])).collect();
    let Some(order) = permutation(tour, cities.len()) else {
        return Some(0.0);
    };
    let length: f64 = (0..order.len())
        .map(|i| {
            let (a, b) = (cities[order[i]], cities[order[(i + 1) % order.len()]]);
            (a.0 - b.0).hypot(a.1 - b.1)
        })
        .sum();
    if length <= 0.0 {
        return Some(0.0);
    }
    Some((reference / length).min(1.0))
}

/// Parses `tour` as a permutation of `0..n`.
fn permutation(tour: &str, n: usize) -> Option<Vec<usize>> {
    let order: Vec<usize> = tour
        .split_whitespace()
        .map(|v| v.parse::<usize>().ok())
        .collect::<Option<_>>()?;
    let mut seen = vec![false; n];
    for &city in &order {
        let slot = seen.get_mut(city)?;
        if *slot {
            return None;
        }
        *slot = true;
    }
    (order.len() == n).then_some(order)
}

fn accuracy(labels: &str, output: &str) -> Option<f64> {
    let truth = lines(labels);
    // Answer lines are positional, so blank answers count (as wrong).
    let answers: Vec<&str> = output.lines().map(str::trim).collect();
    if truth.is_empty() || answers.len() < truth.len() {
        return None;
    }
    let correct = truth.iter().zip(&answers).filter(|(t, a)| t == a).count();
    Some(correct as f64 / truth.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(metric: Metric, inputs: &str, labels: &str, output: &str) -> Option<f64> {
        metric.score(inputs, labels, output).map(Score::get)
    }

    #[test]
    fn parses_names() {
        assert_eq!(Metric::parse("r2"), Some(Metric::R2));
        assert_eq!(Metric::parse("tour_ratio"), Some(Metric::TourRatio));
        assert_eq!(Metric::parse("accuracy"), Some(Metric::Accuracy));
        assert_eq!(Metric::parse("R2"), None);
    }

    #[test]
    fn r2_perfect_mean_and_worse() {
        let labels = "1\n2\n3\n4\n";
        assert_eq!(score(Metric::R2, "", labels, "1\n2\n3\n4\n"), Some(1.0));
        assert_eq!(
            score(Metric::R2, "", labels, "2.5\n2.5\n2.5\n2.5\n"),
            Some(0.0)
        );
        assert_eq!(
            score(Metric::R2, "", labels, "4\n3\n2\n1\n"),
            Some(0.0),
            "clamped"
        );
        let partial = score(Metric::R2, "", labels, "1\n2\n3\n5\n").expect("valid");
        assert!((partial - 0.8).abs() < 1e-12, "1 - 1/5");
    }

    #[test]
    fn r2_rejects_malformed_output() {
        let labels = "1\n2\n";
        for bad in ["1\n", "1\n2\n3\n", "1\nx\n", "1\nnan\n", "1\ninf\n", ""] {
            assert_eq!(score(Metric::R2, "", labels, bad), None, "{bad:?}");
        }
        assert_eq!(
            score(Metric::R2, "", "5\n5\n", "5\n5\n"),
            Some(1.0),
            "constant labels"
        );
        assert_eq!(score(Metric::R2, "", "5\n5\n", "5\n6\n"), Some(0.0));
        assert_eq!(
            score(Metric::R2, "", labels, "1e308\n-1e308\n"),
            Some(0.0),
            "overflow"
        );
    }

    #[test]
    fn tour_ratio_scores_against_reference() {
        // Unit square: the perimeter tour has length 4.
        let square = "0 0 1 0 1 1 0 1\n";
        assert_eq!(
            score(Metric::TourRatio, square, "4\n", "0 1 2 3\n"),
            Some(1.0)
        );
        let crossed = score(Metric::TourRatio, square, "4\n", "0 2 1 3\n").expect("valid");
        assert!((crossed - 4.0 / (2.0 + 2.0 * 2f64.sqrt())).abs() < 1e-12);
        assert_eq!(
            score(Metric::TourRatio, square, "3\n", "0 1 2 3\n"),
            Some(0.75)
        );
        assert_eq!(
            score(Metric::TourRatio, square, "5\n", "0 1 2 3\n"),
            Some(1.0),
            "capped"
        );
    }

    #[test]
    fn invalid_tours_score_zero_but_malformed_files_are_none() {
        let square = "0 0 1 0 1 1 0 1\n";
        for bad_tour in [
            "0 1 2\n",
            "0 1 2 2\n",
            "0 1 2 4\n",
            "0 1 2 x\n",
            "0 1 2 3 0\n",
        ] {
            assert_eq!(
                score(Metric::TourRatio, square, "4\n", bad_tour),
                Some(0.0),
                "{bad_tour:?}"
            );
        }
        let two = "0 0 1 0 1 1 0 1\n0 0 2 0 2 2 0 2\n";
        assert_eq!(
            score(Metric::TourRatio, two, "4\n8\n", "0 1 2 3\n"),
            None,
            "missing line"
        );
        assert_eq!(
            score(Metric::TourRatio, "0 0 1\n", "4\n", "0\n"),
            None,
            "odd coordinates"
        );
        assert_eq!(
            score(Metric::TourRatio, square, "0\n", "0 1 2 3\n"),
            None,
            "bad reference"
        );
        let mean = score(Metric::TourRatio, two, "4\n8\n", "0 1 2 3\n0 1 2\n").expect("valid");
        assert_eq!(mean, 0.5);
    }

    #[test]
    fn accuracy_counts_trimmed_matches() {
        let labels = "42\nseven\n-3\n";
        assert_eq!(
            score(Metric::Accuracy, "", labels, "42\n seven \n-3\n"),
            Some(1.0)
        );
        let two_thirds = score(Metric::Accuracy, "", labels, "42\nseven\n3\n").expect("valid");
        assert!((two_thirds - 2.0 / 3.0).abs() < 1e-12);
        assert_eq!(
            score(Metric::Accuracy, "", labels, "42\n\n-3\n").map(|v| v > 0.6),
            Some(true)
        );
        assert_eq!(
            score(Metric::Accuracy, "", labels, "42\n"),
            None,
            "too few lines"
        );
        assert_eq!(score(Metric::Accuracy, "", "", "x\n"), None);
    }
}
