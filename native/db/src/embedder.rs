//! Model2Vec-based Rust embedder. Replaces the Python SentenceTransformer
//! path so callers can produce float32 sentence embeddings without torch.

use model2vec_rs::model::StaticModel;

use crate::error::{DbError, DbResult};

pub struct Embedder {
    model: StaticModel,
    dim: usize,
}

impl Embedder {
    /// Load a Model2Vec checkpoint from either a local path or a
    /// HuggingFace repo ID (e.g. ``"minishlab/potion-base-8M"``). If the
    /// argument points to an existing directory it is used directly;
    /// otherwise it is resolved against the HuggingFace Hub and cached in
    /// ``~/.cache/huggingface``.
    pub fn load(repo_or_path: &str) -> DbResult<Self> {
        let model = StaticModel::from_pretrained(repo_or_path, None, None, None)
            .map_err(|e| DbError::Other(format!("model2vec load failed: {e}")))?;
        // Probe the output dimension. There is no public dim() accessor on
        // StaticModel in 0.1.x, so encode a trivial string once and read
        // the vector length. Cost: one pass over the embedding table.
        let probe = model.encode_single("probe");
        if probe.is_empty() {
            return Err(DbError::Other(
                "embedder produced empty vector on probe".into(),
            ));
        }
        let dim = probe.len();
        Ok(Self { model, dim })
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Encode a batch of sentences into contiguous little-endian float32
    /// bytes of length ``texts.len() * dim * 4``. The upstream library
    /// already mean-pools and (when the config says so) L2-normalizes
    /// each vector.
    pub fn encode_bytes(&self, texts: &[String]) -> DbResult<Vec<u8>> {
        let vectors = self.model.encode(texts);
        let mut out = Vec::with_capacity(texts.len() * self.dim * 4);
        for v in vectors {
            if v.len() != self.dim {
                return Err(DbError::Other(format!(
                    "embedder returned vector of length {} (expected {})",
                    v.len(),
                    self.dim
                )));
            }
            for x in v {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        Ok(out)
    }
}
