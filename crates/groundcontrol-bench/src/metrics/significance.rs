//! Statistical significance testing for Information Retrieval benchmarking.
//!
//! Provides paired Student's t-test and Wilcoxon signed-rank test to determine
//! whether retrieval quality deltas (NDCG, MRR, Recall) between two systems
//! (e.g., Fast Hybrid vs BM25 Baseline) are statistically significant.

use serde::{Deserialize, Serialize};

/// Result of a statistical significance hypothesis test.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignificanceResult {
    /// Test name ("Paired t-test" or "Wilcoxon signed-rank").
    pub test_name: String,
    /// Test statistic (t-value or z-value).
    pub statistic: f64,
    /// Two-tailed p-value.
    pub p_value: f64,
    /// Significance marker: "***" (p < 0.001), "**" (p < 0.01), "*" (p < 0.05), or "n.s."
    pub significance_marker: &'static str,
    /// Mean difference (system_a - system_b).
    pub mean_difference: f64,
    /// Sample size (number of evaluated queries).
    pub sample_size: usize,
}

/// Evaluator for statistical significance comparisons.
pub struct SignificanceEvaluator;

impl SignificanceEvaluator {
    /// Calculate paired Student's t-test between system A and system B metrics across identical queries.
    pub fn paired_t_test(system_a: &[f64], system_b: &[f64]) -> Option<SignificanceResult> {
        let n = system_a.len();
        if n < 2 || n != system_b.len() {
            return None;
        }

        let diffs: Vec<f64> = system_a.iter().zip(system_b.iter()).map(|(a, b)| a - b).collect();
        let mean_diff = diffs.iter().sum::<f64>() / n as f64;

        let variance = diffs.iter().map(|d| (d - mean_diff).powi(2)).sum::<f64>() / (n - 1) as f64;

        if variance <= f64::EPSILON {
            // No variance between paired samples
            return Some(SignificanceResult {
                test_name: "Paired t-test".to_string(),
                statistic: 0.0,
                p_value: if mean_diff == 0.0 { 1.0 } else { 0.0 },
                significance_marker: if mean_diff == 0.0 { "n.s." } else { "***" },
                mean_difference: mean_diff,
                sample_size: n,
            });
        }

        let std_err = (variance / n as f64).sqrt();
        let t_stat = mean_diff / std_err;
        let df = (n - 1) as f64;

        // Approximate two-tailed p-value using Student's t distribution approximation
        let p_val = t_distribution_two_tailed_p(t_stat.abs(), df);
        let marker = get_significance_marker(p_val);

        Some(SignificanceResult {
            test_name: "Paired t-test".to_string(),
            statistic: t_stat,
            p_value: p_val,
            significance_marker: marker,
            mean_difference: mean_diff,
            sample_size: n,
        })
    }

    /// Calculate Wilcoxon signed-rank test for paired non-normal metric distributions.
    pub fn wilcoxon_signed_rank(system_a: &[f64], system_b: &[f64]) -> Option<SignificanceResult> {
        let n_orig = system_a.len();
        if n_orig < 5 || n_orig != system_b.len() {
            return None;
        }

        // Filter out zero differences (ties)
        let mut diffs: Vec<f64> = system_a
            .iter()
            .zip(system_b.iter())
            .map(|(a, b)| a - b)
            .filter(|&d| d.abs() > 1e-9)
            .collect();

        let n = diffs.len();
        if n < 5 {
            return None;
        }

        let mean_diff = diffs.iter().sum::<f64>() / diffs.len() as f64;

        // Sort by absolute difference
        diffs.sort_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap());

        // Assign ranks with average rank for ties
        let mut ranks = vec![0.0; n];
        let mut i = 0;
        while i < n {
            let mut j = i;
            let val = diffs[i].abs();
            while j < n && (diffs[j].abs() - val).abs() < 1e-9 {
                j += 1;
            }
            // Average rank for [i..j)
            let avg_rank = (i + 1 + j) as f64 / 2.0;
            for r in &mut ranks[i..j] {
                *r = avg_rank;
            }
            i = j;
        }

        // Compute W = sum of signed ranks
        let mut w = 0.0;
        for (idx, &d) in diffs.iter().enumerate() {
            if d > 0.0 {
                w += ranks[idx];
            } else {
                w -= ranks[idx];
            }
        }

        // Standard normal approximation for n >= 10
        let n_f = n as f64;
        let std_w = ((n_f * (n_f + 1.0) * (2.0 * n_f + 1.0)) / 6.0).sqrt();

        // Continuity correction
        let z = if w > 0.0 {
            (w - 0.5) / std_w
        } else if w < 0.0 {
            (w + 0.5) / std_w
        } else {
            0.0
        };

        let p_val = normal_two_tailed_p(z.abs());
        let marker = get_significance_marker(p_val);

        Some(SignificanceResult {
            test_name: "Wilcoxon signed-rank".to_string(),
            statistic: z,
            p_value: p_val,
            significance_marker: marker,
            mean_difference: mean_diff,
            sample_size: n_orig,
        })
    }
}

/// Helper function to assign standard arXiv significance markers.
fn get_significance_marker(p: f64) -> &'static str {
    if p < 0.001 {
        "***"
    } else if p < 0.01 {
        "**"
    } else if p < 0.05 {
        "*"
    } else {
        "n.s."
    }
}

/// Standard normal CDF approximation (Abramowitz and Stegun formula 7.1.26).
fn normal_cdf(x: f64) -> f64 {
    let b1 = 0.319381530;
    let b2 = -0.356563782;
    let b3 = 1.781477937;
    let b4 = -1.821255978;
    let b5 = 1.330274429;
    let p = 0.2316419;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let abs_x = x.abs();

    let t = 1.0 / (1.0 + p * abs_x);
    let poly = t * (b1 + t * (b2 + t * (b3 + t * (b4 + t * b5))));
    let normal_pdf = (-0.5 * abs_x * abs_x).exp() / (2.0 * std::f64::consts::PI).sqrt();
    let cdf = 1.0 - normal_pdf * poly;

    if sign < 0.0 {
        1.0 - cdf
    } else {
        cdf
    }
}

/// Two-tailed p-value from standard normal distribution.
fn normal_two_tailed_p(abs_z: f64) -> f64 {
    2.0 * (1.0 - normal_cdf(abs_z)).clamp(0.0, 1.0)
}

/// Approximate two-tailed p-value for Student's t distribution with `df` degrees of freedom.
fn t_distribution_two_tailed_p(abs_t: f64, df: f64) -> f64 {
    // For large df (df >= 30), t-distribution approaches standard normal
    if df >= 30.0 {
        return normal_two_tailed_p(abs_t);
    }

    // Hill's approximation (CACM Algorithm 395) for smaller df
    let x = df / (df + abs_t * abs_t);
    // Rough approximation via transformed normal approximation
    let a = df - 0.5;
    let b = 48.0 * a * a;
    let z = (a * (1.0 / x).ln()).sqrt();
    let z_corr = z - (3.0 / b) * (z.powi(3) + 3.0 * z);
    normal_two_tailed_p(z_corr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_series_t_test() {
        let a = vec![0.8, 0.9, 0.7, 0.85, 0.95];
        let b = vec![0.8, 0.9, 0.7, 0.85, 0.95];
        let res = SignificanceEvaluator::paired_t_test(&a, &b).unwrap();
        assert_eq!(res.mean_difference, 0.0);
        assert_eq!(res.p_value, 1.0);
        assert_eq!(res.significance_marker, "n.s.");
    }

    #[test]
    fn test_significant_improvement_t_test() {
        let baseline = vec![0.2, 0.3, 0.25, 0.15, 0.3, 0.2, 0.18, 0.22, 0.25, 0.2];
        let proposed = vec![0.9, 0.85, 0.92, 0.88, 0.95, 0.89, 0.91, 0.87, 0.94, 0.9];
        let res = SignificanceEvaluator::paired_t_test(&proposed, &baseline).unwrap();
        assert!(res.mean_difference > 0.6);
        assert!(res.p_value < 0.001);
        assert_eq!(res.significance_marker, "***");
    }

    #[test]
    fn test_wilcoxon_test() {
        let baseline = vec![0.5, 0.4, 0.6, 0.55, 0.45, 0.5, 0.48, 0.52, 0.49, 0.51];
        let proposed = vec![0.8, 0.75, 0.85, 0.9, 0.78, 0.82, 0.88, 0.84, 0.86, 0.81];
        let res = SignificanceEvaluator::wilcoxon_signed_rank(&proposed, &baseline).unwrap();
        assert!(res.mean_difference > 0.3);
        assert!(res.p_value < 0.01);
        assert!(res.significance_marker == "**" || res.significance_marker == "***");
    }
}
