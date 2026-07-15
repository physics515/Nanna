//! Tool schema definitions

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Tool definition for LLM function calling
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Vec<ToolParameter>,
    /// Optional JSON Schema describing the tool's output format.
    /// Used for documentation and post-processing optimization.
    /// Tools returning structured JSON can declare their schema here
    /// so context compression can parse/truncate outputs intelligently.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
}

/// Tool parameter definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParameter {
    pub name: String,
    pub description: String,
    pub param_type: ParameterType,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
}

/// Parameter types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
}

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: Vec::new(),
            output_schema: None,
        }
    }

    /// Set the output schema for this tool definition.
    #[must_use]
    pub fn with_output_schema(mut self, schema: Value) -> Self {
        self.output_schema = Some(schema);
        self
    }

    #[must_use] 
    pub fn param(mut self, param: ToolParameter) -> Self {
        self.parameters.push(param);
        self
    }

    pub fn string_param(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        required: bool,
    ) -> Self {
        self.parameters.push(ToolParameter {
            name: name.into(),
            description: description.into(),
            param_type: ParameterType::String,
            required,
            default: None,
            enum_values: None,
        });
        self
    }

    pub fn int_param(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        required: bool,
    ) -> Self {
        self.parameters.push(ToolParameter {
            name: name.into(),
            description: description.into(),
            param_type: ParameterType::Integer,
            required,
            default: None,
            enum_values: None,
        });
        self
    }

    pub fn integer_param(
        self,
        name: impl Into<String>,
        description: impl Into<String>,
        required: bool,
    ) -> Self {
        self.int_param(name, description, required)
    }

    pub fn array_param(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        required: bool,
    ) -> Self {
        self.parameters.push(ToolParameter {
            name: name.into(),
            description: description.into(),
            param_type: ParameterType::Array,
            required,
            default: None,
            enum_values: None,
        });
        self
    }

    pub fn enum_param(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        required: bool,
        values: &[&str],
    ) -> Self {
        self.parameters.push(ToolParameter {
            name: name.into(),
            description: description.into(),
            param_type: ParameterType::String,
            required,
            default: None,
            enum_values: Some(values.iter().map(std::string::ToString::to_string).collect()),
        });
        self
    }

    pub fn bool_param(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        required: bool,
    ) -> Self {
        self.parameters.push(ToolParameter {
            name: name.into(),
            description: description.into(),
            param_type: ParameterType::Boolean,
            required,
            default: None,
            enum_values: None,
        });
        self
    }

    /// Convert to Anthropic tool format
    #[must_use] 
    pub fn to_anthropic_format(&self) -> Value {
        let properties: serde_json::Map<String, Value> = self
            .parameters
            .iter()
            .map(|p| {
                let mut prop = serde_json::Map::new();
                prop.insert(
                    "type".to_string(),
                    Value::String(match p.param_type {
                        ParameterType::String => "string",
                        ParameterType::Integer => "integer",
                        ParameterType::Number => "number",
                        ParameterType::Boolean => "boolean",
                        ParameterType::Array => "array",
                        ParameterType::Object => "object",
                    }.to_string()),
                );
                prop.insert("description".to_string(), Value::String(p.description.clone()));
                if let Some(ref enums) = p.enum_values {
                    prop.insert(
                        "enum".to_string(),
                        Value::Array(enums.iter().map(|e| Value::String(e.clone())).collect()),
                    );
                }
                (p.name.clone(), Value::Object(prop))
            })
            .collect();

        let required: Vec<Value> = self
            .parameters
            .iter()
            .filter(|p| p.required)
            .map(|p| Value::String(p.name.clone()))
            .collect();

        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "input_schema": {
                "type": "object",
                "properties": properties,
                "required": required
            }
        })
    }

    /// Convert to `OpenAI` tool format
    #[must_use] 
    pub fn to_openai_format(&self) -> Value {
        let properties: serde_json::Map<String, Value> = self
            .parameters
            .iter()
            .map(|p| {
                let mut prop = serde_json::Map::new();
                prop.insert(
                    "type".to_string(),
                    Value::String(match p.param_type {
                        ParameterType::String => "string",
                        ParameterType::Integer => "integer",
                        ParameterType::Number => "number",
                        ParameterType::Boolean => "boolean",
                        ParameterType::Array => "array",
                        ParameterType::Object => "object",
                    }.to_string()),
                );
                prop.insert("description".to_string(), Value::String(p.description.clone()));
                if let Some(ref enums) = p.enum_values {
                    prop.insert(
                        "enum".to_string(),
                        Value::Array(enums.iter().map(|e| Value::String(e.clone())).collect()),
                    );
                }
                (p.name.clone(), Value::Object(prop))
            })
            .collect();

        let required: Vec<Value> = self
            .parameters
            .iter()
            .filter(|p| p.required)
            .map(|p| Value::String(p.name.clone()))
            .collect();

        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": {
                    "type": "object",
                    "properties": properties,
                    "required": required
                }
            }
        })
    }
}
