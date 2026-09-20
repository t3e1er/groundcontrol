/// Attention-weighted mean pooling and L2 normalization over raw output tensor.
pub(crate) fn pool_embeddings(
    batch_size: usize,
    max_len: usize,
    hidden_size: usize,
    flat_attention_mask: &[i64],
    hidden_data: &[f32],
) -> Vec<Vec<f32>> {
    let mut embeddings = Vec::with_capacity(batch_size);

    for b in 0..batch_size {
        let mut sum_vec = vec![0.0f32; hidden_size];
        let mut token_count = 0.0f32;

        for s in 0..max_len {
            if flat_attention_mask[b * max_len + s] == 1 {
                token_count += 1.0;
                let offset = (b * max_len + s) * hidden_size;
                for d in 0..hidden_size {
                    sum_vec[d] += hidden_data[offset + d];
                }
            }
        }

        if token_count > 0.0 {
            for val in &mut sum_vec {
                *val /= token_count;
            }
        }

        let norm: f32 = sum_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for val in &mut sum_vec {
                *val /= norm;
            }
        }

        embeddings.push(sum_vec);
    }

    embeddings
}

/// Compute a document-level embedding by averaging chunk embeddings.
pub fn average_embeddings(embeddings: &[Vec<f32>]) -> Option<Vec<f32>> {
    if embeddings.is_empty() {
        return None;
    }

    let dims = embeddings[0].len();
    let count = embeddings.len() as f32;

    let mut avg = vec![0.0f32; dims];
    for emb in embeddings {
        for (i, &val) in emb.iter().enumerate() {
            avg[i] += val;
        }
    }
    for val in &mut avg {
        *val /= count;
    }

    let norm: f32 = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut avg {
            *val /= norm;
        }
    }

    Some(avg)
}
