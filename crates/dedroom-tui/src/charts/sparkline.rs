//! Unicode sparkline rendering for mini trend charts.
//!
//! Uses 8-level unicode block characters: `▁▂▃▄▅▆▇█`
//! Optimized for rendering small data series in terminal UIs.

/// Sparkline characters from lowest (0) to highest (7).
const SPARK_CHARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Number of levels in the sparkline range (0..=7).
const LEVELS: f64 = 7.0;

/// Render a floating-point series as a sparkline.
pub fn render_sparkline_f64(values: &[f64], max_width: usize) -> String {
    if values.is_empty() {
        return String::new();
    }

    let sampled = if values.len() > max_width {
        downsample_f64(values, max_width)
    } else {
        values.to_vec()
    };

    let min = sampled.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = sampled.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    if (max - min).abs() < f64::EPSILON || !min.is_finite() || !max.is_finite() {
        return std::iter::repeat(SPARK_CHARS[3]).take(sampled.len()).collect();
    }

    let range = max - min;
    sampled
        .iter()
        .map(|&v| {
            let normalized = (v - min) / range;
            let idx = (normalized * LEVELS).round() as usize;
            let idx = idx.min(SPARK_CHARS.len() - 1);
            SPARK_CHARS[idx]
        })
        .collect()
}

fn downsample_f64(data: &[f64], target: usize) -> Vec<f64> {
    if data.len() <= target {
        return data.to_vec();
    }
    let bucket_size = data.len() / target;
    (0..target)
        .map(|i| {
            let start = i * bucket_size;
            let end = if i == target - 1 {
                data.len()
            } else {
                start + bucket_size
            };
            let sum: f64 = data[start..end].iter().sum();
            let count = (end - start).max(1);
            sum / count as f64
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f64() {
        let line = render_sparkline_f64(&[0.0, 0.5, 1.0], 30);
        // round(0.5 * 7) = round(3.5) = 4 (bankers rounding) → SPARK_CHARS[4] = '▅'
        assert_eq!(line, "▁▅█");
    }
}
