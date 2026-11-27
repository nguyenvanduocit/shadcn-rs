//! Sentence embeddings with candle: ModernBERT (granite-embedding-small-english-r2), CLS pooling,
//! L2-normalised. Runs on Metal when available.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::modernbert::{Config, ModernBert};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::config::{self, EMBED_TOKENS, MODEL, MODEL_FILES};

pub struct Embedder {
    model: ModernBert,
    tok: Tokenizer,
    device: Device,
    pub dim: usize,
}

fn hf_cache_dir(model: &str) -> Option<PathBuf> {
    let base = std::env::var_os("HF_HOME")
        .map(|h| PathBuf::from(h).join("hub"))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/huggingface/hub")))?;
    let snaps = base.join(format!("models--{}", model.replace('/', "--"))).join("snapshots");
    std::fs::read_dir(snaps).ok()?.flatten().map(|e| e.path())
        .find(|p| MODEL_FILES.iter().all(|f| p.join(f).exists()))
}

/// Model directory: our own copy, else the Hugging Face cache, else download.
pub fn model_dir() -> Result<PathBuf> {
    model_dir_for(MODEL)
}

pub fn model_dir_for(model: &str) -> Result<PathBuf> {
    let own = config::home().join("model").join(model.replace('/', "--"));
    if MODEL_FILES.iter().all(|f| own.join(f).exists()) {
        return Ok(own);
    }
    if let Some(p) = hf_cache_dir(model) {
        return Ok(p);
    }
    std::fs::create_dir_all(&own)?;
    for f in MODEL_FILES {
        let url = format!("https://huggingface.co/{model}/resolve/main/{f}");
        eprintln!("downloading {url}");
        let mut resp = ureq::get(&url).call().with_context(|| url.clone())?;
        let mut out = std::fs::File::create(own.join(format!("{f}.part")))?;
        std::io::copy(&mut resp.body_mut().as_reader(), &mut out)?;
        std::fs::rename(own.join(format!("{f}.part")), own.join(f))?;
    }
    Ok(own)
}

impl Embedder {
    pub fn load() -> Result<Self> {
        Self::load_from(&model_dir()?)
    }

    pub fn load_from(dir: &Path) -> Result<Self> {
        let device = Device::new_metal(0).unwrap_or(Device::Cpu);
        let cfg: Config = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)?;
        let mut tok = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| anyhow!("{e}"))?;
        tok.with_truncation(Some(TruncationParams { max_length: EMBED_TOKENS, ..Default::default() }))
            .map_err(|e| anyhow!("{e}"))?;
        tok.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_id: cfg.pad_token_id,
            ..Default::default()
        }));
        // The checkpoint stores the backbone without the `model.` prefix candle expects.
        let vb = load_weights(dir, &device)?
            .rename_f(|n: &str| n.strip_prefix("model.").unwrap_or(n).to_string());
        let model = ModernBert::load(vb, &cfg)?;
        Ok(Embedder { model, tok, device, dim: cfg.hidden_size })
    }

    pub fn device_name(&self) -> &'static str {
        if self.device.is_metal() { "metal" } else { "cpu" }
    }

    pub fn encode(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let enc = self.tok.encode_batch(texts.to_vec(), true).map_err(|e| anyhow!("{e}"))?;
        let b = enc.len();
        let n = enc[0].get_ids().len();
        let ids: Vec<u32> = enc.iter().flat_map(|e| e.get_ids().iter().copied()).collect();
        let mask: Vec<f32> =
            enc.iter().flat_map(|e| e.get_attention_mask().iter().map(|&m| m as f32)).collect();
        let ids = Tensor::from_vec(ids, (b, n), &self.device)?;
        let mask = Tensor::from_vec(mask, (b, n), &self.device)?;
        let hidden = self.model.forward(&ids, &mask)?; // (b, n, d)
        let cls = hidden.i((.., 0, ..))?.to_dtype(DType::F32)?;
        let norm = cls.sqr()?.sum_keepdim(1)?.sqrt()?.clamp(1e-12, f64::MAX)?;
        let cls = cls.broadcast_div(&norm)?;
        Ok(cls.to_vec2::<f32>()?)
    }
}

/// Load safetensors by reading them into memory (no mmap, which candle exposes as `unsafe`).
pub fn load_weights(dir: &Path, device: &Device) -> Result<VarBuilder<'static>> {
    let bytes = std::fs::read(dir.join("model.safetensors"))?;
    let tensors = candle_core::safetensors::load_buffer(&bytes, device)?;
    let tensors = tensors
        .into_iter()
        .map(|(k, v)| Ok((k, v.to_dtype(DType::F32)?)))
        .collect::<Result<std::collections::HashMap<_, _>>>()?;
    Ok(VarBuilder::from_tensors(tensors, DType::F32, device))
}
