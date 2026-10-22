use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

use crate::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkflowLoraInput {
    pub file_name: String,
    pub weight: f32,
    pub clip_weight: Option<f32>,
}

impl Default for WorkflowLoraInput {
    fn default() -> Self {
        Self {
            file_name: String::new(),
            weight: 0.75,
            clip_weight: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowBuildRequest {
    pub workflow: Value,
    pub lora_stack: Vec<WorkflowLoraInput>,
    pub checkpoint_node: Option<String>,
    pub image_width: u32,
    pub image_height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowBuildResult {
    pub workflow: Value,
    pub lora_node_ids: Vec<String>,
    pub checkpoint_node: String,
}

#[derive(Debug, Clone)]
pub struct RaphaelWorkflowRequest {
    pub checkpoint_name: String,
    pub use_unet_loader: bool,
    pub positive_prompt: String,
    pub negative_prompt: String,
    pub seed: u64,
    pub image_width: u32,
    pub image_height: u32,
    pub lora_stack: Vec<WorkflowLoraInput>,
}

/// Built-in ComfyUI graph mirrored from Raphael Image Generator.
/// An explicit user-supplied API workflow still takes precedence in the app.
pub fn build_raphael_workflow(request: RaphaelWorkflowRequest) -> AppResult<WorkflowBuildResult> {
    validate_image_dimension(request.image_width, "width")?;
    validate_image_dimension(request.image_height, "height")?;
    if request.checkpoint_name.trim().is_empty() {
        return Err(AppError::ComfyUi("the selected Registry checkpoint has no ComfyUI model path".into()));
    }

    let base_loader = if request.use_unet_loader {
        json!({
            "class_type": "UNETLoader",
            "inputs": {
                "unet_name": request.checkpoint_name,
                "weight_dtype": "default"
            }
        })
    } else {
        json!({
            "class_type": "CheckpointLoaderSimple",
            "inputs": { "ckpt_name": request.checkpoint_name }
        })
    };

    let mut nodes = Map::<String, Value>::new();
    nodes.insert("13".into(), base_loader);
    // Keep the exact external encoder and VAE nodes used by Raphael Image Generator.
    nodes.insert("4".into(), json!({
        "class_type": "CLIPLoader",
        "inputs": {
            "clip_name": "anima\\oneObsession_anima29BV1_txt.safetensors",
            "type": "stable_diffusion",
            "device": "default"
        }
    }));
    nodes.insert("9".into(), json!({
        "class_type": "VAELoader",
        "inputs": { "vae_name": "anima\\qwen_image_vae.safetensors" }
    }));
    nodes.insert("5".into(), json!({
        "class_type": "CLIPTextEncode",
        "inputs": { "clip": ["4", 0], "text": request.positive_prompt }
    }));
    nodes.insert("6".into(), json!({
        "class_type": "CLIPTextEncode",
        "inputs": { "clip": ["4", 0], "text": request.negative_prompt }
    }));
    nodes.insert("8".into(), json!({
        "class_type": "EmptyLatentImage",
        "inputs": {
            "width": request.image_width,
            "height": request.image_height,
            "batch_size": 1
        }
    }));

    let mut seen_files = HashSet::new();
    let mut lora_node_ids = Vec::new();
    let mut last_model = "13".to_string();
    let mut next_id = 14_u64;
    for lora in request.lora_stack {
        let file_name = lora.file_name.trim().to_string();
        if file_name.is_empty() {
            return Err(AppError::ComfyUi("built-in workflow received an empty LoRA file name".into()));
        }
        if !seen_files.insert(file_name.clone()) {
            continue;
        }
        let node_id = next_id.to_string();
        nodes.insert(node_id.clone(), json!({
            "class_type": "LoraLoaderModelOnly",
            "inputs": {
                "model": [last_model, 0],
                "lora_name": file_name,
                "strength_model": lora.weight
            }
        }));
        last_model = node_id.clone();
        lora_node_ids.push(node_id);
        next_id += 1;
    }

    nodes.insert("7".into(), json!({
        "class_type": "KSampler",
        "inputs": {
            "model": [last_model, 0],
            "positive": ["5", 0],
            "negative": ["6", 0],
            "latent_image": ["8", 0],
            "seed": request.seed,
            "steps": 28,
            "cfg": 6.5,
            "sampler_name": "euler",
            "scheduler": "normal",
            "denoise": 1.0
        }
    }));
    nodes.insert("10".into(), json!({
        "class_type": "VAEDecode",
        "inputs": { "samples": ["7", 0], "vae": ["9", 0] }
    }));
    nodes.insert("12".into(), json!({
        "class_type": "SaveImage",
        "inputs": { "images": ["10", 0], "filename_prefix": "Raphael" }
    }));

    Ok(WorkflowBuildResult {
        workflow: Value::Object(nodes),
        lora_node_ids,
        checkpoint_node: "13".into(),
    })
}

fn validate_image_dimension(value: u32, label: &str) -> AppResult<()> {
    if !(512..=4096).contains(&value) || value % 8 != 0 {
        return Err(AppError::ComfyUi(format!(
            "image {label} must be between 512 and 4096 and divisible by 8; received {value}"
        )));
    }
    Ok(())
}

fn replace_image_size_placeholders(value: &mut Value, width: u32, height: u32) -> usize {
    let mut replaced = 0;
    match value {
        Value::String(text) => {
            if text == "{{IMAGE_WIDTH}}" { *value = Value::from(width); return 1; }
            if text == "{{IMAGE_HEIGHT}}" { *value = Value::from(height); return 1; }
            if text == "{{IMAGE_SIZE}}" { *value = Value::String(format!("{width}x{height}")); return 1; }
            let original = text.clone();
            *text = text.replace("{{IMAGE_WIDTH}}", &width.to_string())
                .replace("{{IMAGE_HEIGHT}}", &height.to_string())
                .replace("{{IMAGE_SIZE}}", &format!("{width}x{height}"));
            if *text != original { replaced += 1; }
        }
        Value::Array(items) => for item in items { replaced += replace_image_size_placeholders(item, width, height); },
        Value::Object(map) => for item in map.values_mut() { replaced += replace_image_size_placeholders(item, width, height); },
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    replaced
}

fn inject_latent_size(workflow: &mut Value, width: u32, height: u32) -> usize {
    let Value::Object(nodes) = workflow else { return 0; };
    let mut count = 0;
    for node in nodes.values_mut() {
        let class_type = node.get("class_type").and_then(Value::as_str).unwrap_or_default().to_ascii_lowercase();
        if !class_type.contains("empty") || !class_type.contains("latent") { continue; }
        let Some(inputs) = node.get_mut("inputs").and_then(Value::as_object_mut) else { continue; };
        if inputs.contains_key("width") && inputs.contains_key("height") {
            inputs.insert("width".into(), Value::from(width));
            inputs.insert("height".into(), Value::from(height));
            count += 1;
        }
    }
    count
}
fn find_checkpoint_node(workflow: &Map<String, Value>, requested: Option<&str>) -> AppResult<String> {
    if let Some(node_id) = requested {
        let Some(node) = workflow.get(node_id) else {
            return Err(AppError::ComfyUi(format!(
                "workflow builder checkpoint node '{}' does not exist",
                node_id
            )));
        };
        let class_type = node.get("class_type").and_then(Value::as_str).unwrap_or_default();
        if class_type != "CheckpointLoaderSimple" && class_type != "CheckpointLoader" {
            return Err(AppError::ComfyUi(format!(
                "workflow builder checkpoint node '{}' is '{}', expected CheckpointLoaderSimple or CheckpointLoader",
                node_id, class_type
            )));
        }
        return Ok(node_id.to_string());
    }

    workflow
        .iter()
        .find_map(|(id, node)| {
            let class_type = node.get("class_type").and_then(Value::as_str)?;
            if class_type == "CheckpointLoaderSimple" || class_type == "CheckpointLoader" {
                Some(id.clone())
            } else {
                None
            }
        })
        .ok_or_else(|| {
            AppError::ComfyUi(
                "workflow builder could not find a CheckpointLoaderSimple/CheckpointLoader node; set checkpoint_node explicitly"
                    .into(),
            )
        })
}

fn next_numeric_node_id(workflow: &Map<String, Value>) -> i64 {
    workflow
        .keys()
        .filter_map(|key| key.parse::<i64>().ok())
        .max()
        .unwrap_or(0)
        + 1
}

fn replace_exact_refs_in_original_nodes(
    nodes: &mut Map<String, Value>,
    original_node_ids: &HashSet<String>,
    checkpoint_node: &str,
    output_index: usize,
    replacement: &[Value],
) {
    for node_id in original_node_ids {
        if let Some(node) = nodes.get_mut(node_id) {
            replace_exact_refs(node, checkpoint_node, output_index, replacement);
        }
    }
}

fn replace_exact_refs(value: &mut Value, checkpoint_node: &str, output_index: usize, replacement: &[Value]) {
    match value {
        Value::Array(items) if items.len() == 2
            && items[0].as_str() == Some(checkpoint_node)
            && items[1].as_u64() == Some(output_index as u64) =>
        {
            *value = Value::Array(replacement.to_vec());
        }
        Value::Array(items) => {
            for item in items {
                replace_exact_refs(item, checkpoint_node, output_index, replacement);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                replace_exact_refs(item, checkpoint_node, output_index, replacement);
            }
        }
        _ => {}
    }
}

pub fn build_workflow(request: WorkflowBuildRequest) -> AppResult<WorkflowBuildResult> {
    validate_image_dimension(request.image_width, "width")?;
    validate_image_dimension(request.image_height, "height")?;
    let mut workflow = request.workflow;
    let placeholder_count = replace_image_size_placeholders(&mut workflow, request.image_width, request.image_height);
    let latent_count = inject_latent_size(&mut workflow, request.image_width, request.image_height);
    if placeholder_count == 0 && latent_count == 0 {
        return Err(AppError::ComfyUi("workflow has no image-size injection point; add {{IMAGE_WIDTH}}/{{IMAGE_HEIGHT}} placeholders or an Empty*Latent node with width and height inputs".into()));
    }
    let Value::Object(ref mut nodes) = workflow else {
        return Err(AppError::ComfyUi(
            "workflow builder requires a ComfyUI API-format object".into(),
        ));
    };

    let checkpoint_node = find_checkpoint_node(nodes, request.checkpoint_node.as_deref())?;

    let mut seen_files = HashSet::new();
    let mut stack = Vec::new();
    for lora in request.lora_stack {
        let file_name = lora.file_name.trim().to_string();
        if file_name.is_empty() {
            return Err(AppError::ComfyUi("workflow builder received a LoRA with an empty file name".into()));
        }
        if !seen_files.insert(file_name.clone()) {
            continue;
        }
        let weight = lora.weight.clamp(-10.0, 10.0);
        let clip_weight = lora.clip_weight.unwrap_or(weight).clamp(-10.0, 10.0);
        stack.push(WorkflowLoraInput {
            file_name,
            weight,
            clip_weight: Some(clip_weight),
        });
    }

    if stack.is_empty() {
        return Ok(WorkflowBuildResult {
            workflow,
            lora_node_ids: Vec::new(),
            checkpoint_node,
        });
    }

    let mut next_id = next_numeric_node_id(nodes);
    let original_node_ids = nodes.keys().cloned().collect::<HashSet<_>>();
    let mut previous_node = checkpoint_node.clone();
    let mut created_nodes = Vec::new();

    for lora in stack {
        let node_id = next_id.to_string();
        next_id += 1;

        nodes.insert(
            node_id.clone(),
            json!({
                "class_type": "LoraLoader",
                "inputs": {
                    "model": [previous_node.clone(), 0],
                    "clip": [previous_node.clone(), 1],
                    "lora_name": lora.file_name,
                    "strength_model": lora.weight,
                    "strength_clip": lora.clip_weight.unwrap_or(lora.weight),
                }
            }),
        );

        previous_node = node_id.clone();
        created_nodes.push(node_id);
    }

    let final_model_ref = vec![Value::String(previous_node.clone()), Value::from(0_u64)];
    let final_clip_ref = vec![Value::String(previous_node.clone()), Value::from(1_u64)];

    if let Value::Object(nodes) = &mut workflow {
        replace_exact_refs_in_original_nodes(
            nodes,
            &original_node_ids,
            &checkpoint_node,
            0,
            &final_model_ref,
        );
        replace_exact_refs_in_original_nodes(
            nodes,
            &original_node_ids,
            &checkpoint_node,
            1,
            &final_clip_ref,
        );
    }

    Ok(WorkflowBuildResult {
        workflow,
        lora_node_ids: created_nodes,
        checkpoint_node,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_workflow() -> Value {
        json!({
            "1": {
                "class_type": "CheckpointLoaderSimple",
                "inputs": { "ckpt_name": "base.safetensors" }
            },
            "2": {
                "class_type": "CLIPTextEncode",
                "inputs": { "clip": ["1", 1], "text": "positive" }
            },
            "3": {
                "class_type": "CLIPTextEncode",
                "inputs": { "clip": ["1", 1], "text": "negative" }
            },
            "4": {
                "class_type": "KSampler",
                "inputs": {
                    "model": ["1", 0],
                    "positive": ["2", 0],
                    "negative": ["3", 0],
                    "latent_image": ["5", 0],
                    "seed": 1
                }
            },
            "5": {
                "class_type": "EmptyLatentImage",
                "inputs": {
                    "width": 1024,
                    "height": 1024,
                    "batch_size": 1
                }
            }
        })
    }

    #[test]
    fn built_in_raphael_workflow_uses_checkpoint_and_model_only_loras() {
        let result = build_raphael_workflow(RaphaelWorkflowRequest {
            checkpoint_name: "anima\\miaomiaoRealskin_anima13.safetensors".into(),
            use_unet_loader: true,
            positive_prompt: "aetheria_trigger, a hero at dusk".into(),
            negative_prompt: "blurry".into(),
            seed: 123,
            image_width: 1024,
            image_height: 1024,
            lora_stack: vec![
                WorkflowLoraInput { file_name: "Anima\\Aetheria.safetensors".into(), weight: 0.85, clip_weight: None },
            ],
        }).unwrap();

        let workflow = result.workflow.as_object().unwrap();
        assert_eq!(workflow["13"]["class_type"], "UNETLoader");
        assert_eq!(workflow["13"]["inputs"]["unet_name"], "anima\\miaomiaoRealskin_anima13.safetensors");
        assert_eq!(workflow["4"]["class_type"], "CLIPLoader");
        assert_eq!(workflow["9"]["class_type"], "VAELoader");
        assert_eq!(workflow["5"]["inputs"]["text"], "aetheria_trigger, a hero at dusk");
        assert_eq!(workflow["14"]["class_type"], "LoraLoaderModelOnly");
        assert_eq!(workflow["14"]["inputs"]["model"], json!(["13", 0]));
        assert_eq!(workflow["7"]["inputs"]["model"], json!(["14", 0]));
        assert_eq!(workflow["7"]["inputs"]["seed"], 123);
        assert_eq!(workflow["8"]["inputs"]["width"], 1024);
        assert_eq!(result.lora_node_ids, vec!["14"]);
    }

    #[test]
    fn built_in_raphael_workflow_uses_full_checkpoint_loader_when_needed() {
        let result = build_raphael_workflow(RaphaelWorkflowRequest {
            checkpoint_name: "example.safetensors".into(),
            use_unet_loader: false,
            positive_prompt: "positive".into(),
            negative_prompt: "negative".into(),
            seed: 1,
            image_width: 1024,
            image_height: 1024,
            lora_stack: Vec::new(),
        }).unwrap();
        assert_eq!(result.workflow["13"]["class_type"], "CheckpointLoaderSimple");
        assert_eq!(result.workflow["7"]["inputs"]["model"], json!(["13", 0]));
        assert_eq!(result.workflow["10"]["inputs"]["vae"], json!(["9", 0]));
    }

    #[test]
    fn builds_arbitrary_lora_chain() {
        let result = build_workflow(WorkflowBuildRequest {
            workflow: base_workflow(),
            checkpoint_node: None,
            image_width: 1024,
            image_height: 1024,
            lora_stack: vec![
                WorkflowLoraInput { file_name: "style.safetensors".into(), weight: 0.7, clip_weight: None },
                WorkflowLoraInput { file_name: "character.safetensors".into(), weight: 0.85, clip_weight: None },
                WorkflowLoraInput { file_name: "pose.safetensors".into(), weight: 0.45, clip_weight: None },
            ],
        }).unwrap();

        assert_eq!(result.lora_node_ids.len(), 3);
        let workflow = result.workflow.as_object().unwrap();

        assert_eq!(workflow["6"]["inputs"]["model"], json!(["1", 0]));
        assert_eq!(workflow["7"]["inputs"]["model"], json!(["6", 0]));
        assert_eq!(workflow["8"]["inputs"]["model"], json!(["7", 0]));
        assert_eq!(workflow["2"]["inputs"]["clip"], json!(["8", 1]));
        assert_eq!(workflow["3"]["inputs"]["clip"], json!(["8", 1]));
        assert_eq!(workflow["4"]["inputs"]["model"], json!(["8", 0]));
        assert_eq!(workflow["5"]["inputs"]["width"], json!(1024));
        assert_eq!(workflow["5"]["inputs"]["height"], json!(1024));
    }

    #[test]
    fn accepts_1920_by_1080_image_size() {
        let result = build_raphael_workflow(RaphaelWorkflowRequest {
            checkpoint_name: "checkpoint.safetensors".into(),
            use_unet_loader: false,
            positive_prompt: "positive".into(),
            negative_prompt: "negative".into(),
            seed: 1,
            image_width: 1920,
            image_height: 1080,
            lora_stack: Vec::new(),
        }).unwrap();
        assert_eq!(result.workflow["8"]["inputs"]["width"], json!(1920));
        assert_eq!(result.workflow["8"]["inputs"]["height"], json!(1080));
    }

    #[test]
    fn accepts_eight_aligned_dimensions_even_if_not_64_aligned() {
        validate_image_dimension(864, "width").unwrap();
        validate_image_dimension(1536, "height").unwrap();
        assert!(validate_image_dimension(863, "width").is_err());
    }

    #[test]
    fn injects_placeholder_image_size() {
        let input = json!({
            "1": {
                "class_type": "CheckpointLoaderSimple",
                "inputs": { "ckpt_name": "base.safetensors" }
            },
            "2": {
                "class_type": "CLIPTextEncode",
                "inputs": { "clip": ["1", 1], "text": "positive" }
            },
            "3": {
                "class_type": "CLIPTextEncode",
                "inputs": { "clip": ["1", 1], "text": "negative" }
            },
            "4": {
                "class_type": "KSampler",
                "inputs": {
                    "model": ["1", 0],
                    "positive": ["2", 0],
                    "negative": ["3", 0],
                    "latent_image": ["5", 0],
                    "seed": 1
                }
            },
            "5": {
                "class_type": "EmptyLatentImage",
                "inputs": {
                    "width": "{{IMAGE_WIDTH}}",
                    "height": "{{IMAGE_HEIGHT}}",
                    "batch_size": 1
                }
            }
        });
        let result = build_workflow(WorkflowBuildRequest {
            workflow: input,
            checkpoint_node: None,
            image_width: 1216,
            image_height: 832,
            lora_stack: Vec::new(),
        }).unwrap();
        let workflow = result.workflow.as_object().unwrap();
        assert_eq!(workflow["5"]["inputs"]["width"], json!(1216));
        assert_eq!(workflow["5"]["inputs"]["height"], json!(832));
    }

    #[test]
    fn rejects_workflows_without_image_size_injection_point() {
        let mut input = base_workflow();
        input.as_object_mut().unwrap().remove("5");
        let result = build_workflow(WorkflowBuildRequest {
            workflow: input,
            checkpoint_node: None,
            image_width: 1024,
            image_height: 1024,
            lora_stack: Vec::new(),
        });
        assert!(result.is_err());
    }

    #[test]
    fn preserves_workflow_when_no_loras_are_selected() {
        let input = base_workflow();
        let result = build_workflow(WorkflowBuildRequest {
            workflow: input.clone(),
            checkpoint_node: None,
            lora_stack: Vec::new(),
            image_width: 1024,
            image_height: 1024,
        }).unwrap();

        assert_eq!(result.workflow, input);
    }

    #[test]
    fn injects_requested_image_size() {
        let mut workflow = base_workflow();
        workflow["5"]["inputs"]["width"] = json!("{{IMAGE_WIDTH}}");
        workflow["5"]["inputs"]["height"] = json!("{{IMAGE_HEIGHT}}");
        let result = build_workflow(WorkflowBuildRequest {
            workflow,
            checkpoint_node: None,
            lora_stack: Vec::new(),
            image_width: 1216,
            image_height: 832,
        }).unwrap();
        assert_eq!(result.workflow["5"]["inputs"]["width"], json!(1216));
        assert_eq!(result.workflow["5"]["inputs"]["height"], json!(832));
    }

    #[test]
    fn rejects_missing_checkpoint_node() {
        let result = build_workflow(WorkflowBuildRequest {
            workflow: base_workflow(),
            checkpoint_node: Some("missing".into()),
            lora_stack: vec![WorkflowLoraInput {
                file_name: "style.safetensors".into(),
                weight: 0.7,
                clip_weight: None,
            }],
            image_width: 1024,
            image_height: 1024,
        });
        assert!(result.is_err());
    }
}
