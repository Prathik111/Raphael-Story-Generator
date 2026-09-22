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
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowBuildResult {
    pub workflow: Value,
    pub lora_node_ids: Vec<String>,
    pub checkpoint_node: String,
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

fn collect_model_clip_refs(value: &Value, checkpoint_node: &str, output_index: usize, out: &mut Vec<Vec<Value>>) {
    match value {
        Value::Array(items) if items.len() == 2 => {
            if items[0].as_str() == Some(checkpoint_node) && items[1].as_u64() == Some(output_index as u64) {
                out.push(items.clone());
                return;
            }
            for item in items {
                collect_model_clip_refs(item, checkpoint_node, output_index, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_model_clip_refs(item, checkpoint_node, output_index, out);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_model_clip_refs(item, checkpoint_node, output_index, out);
            }
        }
        _ => {}
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
    let mut workflow = request.workflow;
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

    replace_exact_refs(
        &mut workflow,
        &checkpoint_node,
        0,
        &final_model_ref,
    );
    replace_exact_refs(
        &mut workflow,
        &checkpoint_node,
        1,
        &final_clip_ref,
    );

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
                    "seed": 1
                }
            }
        })
    }

    #[test]
    fn builds_arbitrary_lora_chain() {
        let result = build_workflow(WorkflowBuildRequest {
            workflow: base_workflow(),
            checkpoint_node: None,
            lora_stack: vec![
                WorkflowLoraInput { file_name: "style.safetensors".into(), weight: 0.7, clip_weight: None },
                WorkflowLoraInput { file_name: "character.safetensors".into(), weight: 0.85, clip_weight: None },
                WorkflowLoraInput { file_name: "pose.safetensors".into(), weight: 0.45, clip_weight: None },
            ],
        }).unwrap();

        assert_eq!(result.lora_node_ids.len(), 3);
        let workflow = result.workflow.as_object().unwrap();

        assert_eq!(workflow["5"]["inputs"]["model"], json!(["1", 0]));
        assert_eq!(workflow["6"]["inputs"]["model"], json!(["5", 0]));
        assert_eq!(workflow["7"]["inputs"]["model"], json!(["6", 0]));
        assert_eq!(workflow["2"]["inputs"]["clip"], json!(["7", 1]));
        assert_eq!(workflow["3"]["inputs"]["clip"], json!(["7", 1]));
        assert_eq!(workflow["4"]["inputs"]["model"], json!(["7", 0]));
    }

    #[test]
    fn preserves_workflow_when_no_loras_are_selected() {
        let input = base_workflow();
        let result = build_workflow(WorkflowBuildRequest {
            workflow: input.clone(),
            checkpoint_node: None,
            lora_stack: Vec::new(),
        }).unwrap();

        assert_eq!(result.workflow, input);
    }

    #[test]
    fn rejects_missing_checkpoint_node() {
        let result = build_workflow(WorkflowBuildRequest {
            workflow: json!({ "1": { "class_type": "KSampler", "inputs": {} } }),
            checkpoint_node: None,
            lora_stack: vec![WorkflowLoraInput {
                file_name: "style.safetensors".into(),
                weight: 0.7,
                clip_weight: None,
            }],
        });
        assert!(result.is_err());
    }
}
