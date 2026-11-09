/**
 * AI Engine module
 * Handles ONNX Runtime sessions and model inference.
 */
use crate::t_common;
use crate::t_gpu::GpuSession as Session;
use image::DynamicImage;
use ndarray::{Array, Array4};
use ort::{
    inputs,
    session::builder::GraphOptimizationLevel,
    value::Value,
};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager};
use tokenizers::Tokenizer;

pub struct AiEngine {
    text_model: Option<Session>,
    vision_model: Option<Session>,
    tokenizer: Option<Tokenizer>,
    text_model_kind: ImageSearchTextModel,
}

const AI_INTRA_THREADS: usize = 2;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageSearchTextModel {
    Default,
}

impl ImageSearchTextModel {
    pub fn from_i64(_value: i64) -> Self {
        Self::Default
    }

    pub fn as_i64(self) -> i64 {
        0
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSearchModelStatus {
    pub active_model: i64,
    pub multilingual_available: bool,
}

#[derive(Debug, Clone)]
struct TextModelPaths {
    model: PathBuf,
    tokenizer: PathBuf,
}

impl AiEngine {
    pub fn new() -> Self {
        Self {
            text_model: None,
            vision_model: None,
            tokenizer: None,
            text_model_kind: ImageSearchTextModel::Default,
        }
    }

    pub fn load_models(&mut self, app: &AppHandle) -> Result<(), String> {
        if self.text_model.is_some() && self.vision_model.is_some() {
            return Ok(());
        }

        println!("Loading AI Models...");

        let resource_dir = Self::resource_model_dir(app)?;
        if !crate::t_models::available("search", &resource_dir) { return Err(crate::t_models::missing("search")); }
        let vision_model_path = resource_dir.join(t_common::AI_VISION_MODEL);
        // Load Vision Model
        if self.vision_model.is_none() {
            let vision_model = Self::load_session(&vision_model_path, "vision")?;
            self.vision_model = Some(vision_model);
        }

        if self.text_model.is_none() {
            self.set_text_model(app, ImageSearchTextModel::Default)?;
        }

        println!("AI Models Loaded Successfully!");
        Ok(())
    }

    pub fn unload_models(&mut self) {
        self.text_model = None;
        self.vision_model = None;
        self.tokenizer = None;
    }

    fn load_session(path: &Path, model_name: &str) -> Result<Session, String> {
        crate::t_gpu::load(path, false, || ort::session::Session::builder()
            .map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| e.to_string())?
            .with_intra_threads(AI_INTRA_THREADS)
            .map_err(|e| e.to_string()))
            .map_err(|e| format!("Failed to load {} model from {:?}: {}", model_name, path, e))
    }

    pub(crate) fn resource_model_dir(app: &AppHandle) -> Result<PathBuf, String> {
        #[cfg(debug_assertions)]
        {
            let manifest_dir = env!("CARGO_MANIFEST_DIR");
            let dev_path = std::path::PathBuf::from(manifest_dir).join("resources/models");
            if dev_path.exists() {
                return Ok(crate::t_models::resolve("search", &dev_path));
            }
        }

        app.path()
            .resolve("models", tauri::path::BaseDirectory::Resource)
            .map(|dir| crate::t_models::resolve("search", &dir))
            .map_err(|e| format!("Failed to resolve resource path: {}", e))
    }

    fn text_model_paths(app: &AppHandle) -> Result<TextModelPaths, String> {
        let model_dir = Self::resource_model_dir(app)?;

        Ok(TextModelPaths {
            model: model_dir.join(t_common::AI_TEXT_MODEL),
            tokenizer: model_dir.join(t_common::AI_TOKENIZER),
        })
    }

    pub fn model_status(&self, _app: &AppHandle) -> ImageSearchModelStatus {
        ImageSearchModelStatus {
            active_model: self.text_model_kind.as_i64(),
            multilingual_available: false,
        }
    }

    pub fn set_text_model(
        &mut self,
        app: &AppHandle,
        model: ImageSearchTextModel,
    ) -> Result<(), String> {
        if self.text_model.is_some() && self.text_model_kind == model {
            return Ok(());
        }

        let paths = Self::text_model_paths(app)?;
        if !paths.model.exists() || !paths.tokenizer.exists() {
            return Err(format!(
                "Image search model files are missing for {:?}",
                model
            ));
        }

        let tokenizer = Tokenizer::from_file(&paths.tokenizer)
            .map_err(|e| format!("Failed to load tokenizer from {:?}: {}", paths.tokenizer, e))?;
        let text_model = Self::load_session(&paths.model, "text")?;

        let previous_text_model = self.text_model.take();
        let previous_tokenizer = self.tokenizer.take();
        let previous_model_kind = self.text_model_kind;
        self.tokenizer = Some(tokenizer);
        self.text_model = Some(text_model);
        self.text_model_kind = model;
        if let Err(error) = self.ensure_embedding_dimensions_match() {
            self.text_model = previous_text_model;
            self.tokenizer = previous_tokenizer;
            self.text_model_kind = previous_model_kind;
            return Err(error);
        }
        Ok(())
    }

    fn ensure_embedding_dimensions_match(&mut self) -> Result<(), String> {
        if self.vision_model.is_none() {
            return Ok(());
        }
        let text_dim = self.encode_text("__lap_embedding_probe__")?.len();
        let vision_dim = self.run_vision_model(Array::zeros((1, 3, 224, 224)))?.len();
        if text_dim != vision_dim {
            return Err(format!(
                "Image search model is incompatible with the bundled vision model (text dimension {}, vision dimension {}). Please reinstall the photo search model from Settings.",
                text_dim, vision_dim
            ));
        }
        Ok(())
    }

    pub fn is_loaded(&self) -> bool {
        self.text_model.is_some() && self.vision_model.is_some() && self.tokenizer.is_some()
    }

    pub fn encode_text(&mut self, text: &str) -> Result<Vec<f32>, String> {
        if self.text_model.is_none() || self.tokenizer.is_none() {
            return Err("Text search model not loaded".to_string());
        }

        let tokenizer = self.tokenizer.as_ref().unwrap();
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| format!("Tokenization error: {}", e))?;

        let input_ids = encoding.get_ids();
        let attention_mask = encoding.get_attention_mask();

        let input_ids_array = Array::from_shape_vec(
            (1, input_ids.len()),
            input_ids.iter().map(|&x| x as i64).collect(),
        )
        .map_err(|e| e.to_string())?;

        let input_ids_value = Value::from_array(input_ids_array).map_err(|e| e.to_string())?;
        let attention_mask_array = Array::from_shape_vec(
            (1, attention_mask.len()),
            attention_mask.iter().map(|&x| x as i64).collect(),
        )
        .map_err(|e| e.to_string())?;
        let attention_mask_value =
            Value::from_array(attention_mask_array).map_err(|e| e.to_string())?;

        let uses_attention_mask = self
            .text_model
            .as_ref()
            .unwrap()
            .inputs
            .iter()
            .any(|input| input.name == "attention_mask");

        let outputs = if uses_attention_mask {
            self.text_model.as_mut().unwrap().run(inputs![
                "input_ids" => input_ids_value,
                "attention_mask" => attention_mask_value,
            ])
        } else {
            self.text_model.as_mut().unwrap().run(inputs![
                "input_ids" => input_ids_value,
            ])
        }
        .map_err(|e| format!("Inference error: {}", e))?;

        let (embedding, first_token_only) = if let Some(vals) = outputs.get("pooler_output") {
            (vals, false)
        } else if let Some(vals) = outputs.get("text_embeds") {
            (vals, false)
        } else if let Some(vals) = outputs.get("last_hidden_state") {
            (vals, true)
        } else {
            (&outputs[0], true)
        };

        Self::extract_text_embedding(embedding, first_token_only)
    }

    fn extract_text_embedding(
        embedding: &ort::value::DynValue,
        first_token_only: bool,
    ) -> Result<Vec<f32>, String> {
        let (shape, embedding_data) = embedding
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract tensor: {}", e))?;

        if first_token_only && shape.len() >= 3 {
            let hidden_size = shape
                .last()
                .copied()
                .filter(|dim| *dim > 0)
                .ok_or_else(|| format!("Invalid text embedding shape: {}", shape))?
                as usize;
            if embedding_data.len() < hidden_size {
                return Err(format!(
                    "Text embedding data is shorter than shape {}",
                    shape
                ));
            }
            return Ok(embedding_data[..hidden_size].to_vec());
        }

        Ok(embedding_data.to_vec())
    }

    pub fn encode_image(&mut self, image_path: &str) -> Result<Vec<f32>, String> {
        if !self.is_loaded() {
            return Err("AI models not loaded".to_string());
        }

        let image_input = self.preprocess_image(image_path)?;
        self.run_vision_model(image_input)
    }

    pub fn encode_image_from_bytes(&mut self, image_bytes: &[u8]) -> Result<Vec<f32>, String> {
        if !self.is_loaded() {
            return Err("AI models not loaded".to_string());
        }

        let img = image::load_from_memory(image_bytes)
            .map_err(|e| format!("Failed to load image from memory: {}", e))?;
        let image_input = self.preprocess_dynamic_image(img)?;

        self.run_vision_model(image_input)
    }

    fn run_vision_model(&mut self, image_input: Array4<f32>) -> Result<Vec<f32>, String> {
        let image_input_value = Value::from_array(image_input).map_err(|e| e.to_string())?;

        let outputs = self
            .vision_model
            .as_mut()
            .unwrap()
            .run(inputs![
                "pixel_values" => image_input_value,
            ])
            .map_err(|e| format!("Inference error: {}", e))?;

        let embedding = if let Some(vals) = outputs.get("pooler_output") {
            vals
        } else if let Some(vals) = outputs.get("image_embeds") {
            vals
        } else {
            &outputs[0]
        };

        let (_, embedding_data) = embedding
            .try_extract_tensor::<f32>()
            .map_err(|e| format!("Failed to extract tensor: {}", e))?;

        Ok(embedding_data.to_vec())
    }

    fn preprocess_image(&self, path: &str) -> Result<Array4<f32>, String> {
        let img = image::open(path).map_err(|e| format!("Failed to open image: {}", e))?;
        self.preprocess_dynamic_image(img)
    }

    fn preprocess_dynamic_image(&self, img: DynamicImage) -> Result<Array4<f32>, String> {
        // resize to 224x224
        let img = img.resize_exact(224, 224, image::imageops::FilterType::Triangle);
        let rgb_img = img.to_rgb8();

        // Normalize
        let mean = [0.48145466, 0.4578275, 0.40821073];
        let std = [0.26862954, 0.26130258, 0.27577711];

        let mut array = Array::zeros((1, 3, 224, 224));

        for (x, y, pixel) in rgb_img.enumerate_pixels() {
            let r = (pixel[0] as f32 / 255.0 - mean[0]) / std[0];
            let g = (pixel[1] as f32 / 255.0 - mean[1]) / std[1];
            let b = (pixel[2] as f32 / 255.0 - mean[2]) / std[2];

            array[[0, 0, y as usize, x as usize]] = r;
            array[[0, 1, y as usize, x as usize]] = g;
            array[[0, 2, y as usize, x as usize]] = b;
        }

        Ok(array)
    }
}

pub struct AiState(pub Mutex<AiEngine>);
