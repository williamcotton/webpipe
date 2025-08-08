use crate::ast::{Config, ConfigValue};
use crate::error::WebPipeError;
use serde_json::Value;
use std::collections::HashMap;
use std::env;
use once_cell::sync::OnceCell;

pub struct ConfigManager {
    configs: HashMap<String, Config>,
}

static GLOBAL_CONFIG: OnceCell<ConfigManager> = OnceCell::new();

impl ConfigManager {
    pub fn new(configs: Vec<Config>) -> Self {
        let configs_map = configs
            .into_iter()
            .map(|config| (config.name.clone(), config))
            .collect();

        Self {
            configs: configs_map,
        }
    }

    pub fn get_config(&self, name: &str) -> Option<&Config> {
        self.configs.get(name)
    }

    pub fn resolve_config_value(&self, value: &ConfigValue) -> Result<Value, WebPipeError> {
        match value {
            ConfigValue::String(s) => Ok(Value::String(s.clone())),
            ConfigValue::Boolean(b) => Ok(Value::Bool(*b)),
            ConfigValue::Number(n) => Ok(Value::Number(serde_json::Number::from(*n))),
            ConfigValue::EnvVar { var, default } => {
                match env::var(var) {
                    Ok(val) => Ok(Value::String(val)),
                    Err(_) => {
                        if let Some(default_val) = default {
                            Ok(Value::String(default_val.clone()))
                        } else {
                            Err(WebPipeError::ConfigError(format!(
                                "Environment variable {} not found and no default provided",
                                var
                            )))
                        }
                    }
                }
            }
        }
    }

    pub fn resolve_config_as_json(&self, name: &str) -> Result<Value, WebPipeError> {
        let config = self.get_config(name)
            .ok_or_else(|| WebPipeError::ConfigError(format!("Config '{}' not found", name)))?;

        let mut result = serde_json::Map::new();
        
        for property in &config.properties {
            let value = self.resolve_config_value(&property.value)?;
            result.insert(property.key.clone(), value);
        }

        Ok(Value::Object(result))
    }

    pub fn get_string_value(&self, config_name: &str, key: &str) -> Result<String, WebPipeError> {
        let config_json = self.resolve_config_as_json(config_name)?;
        
        config_json
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| WebPipeError::ConfigError(format!(
                "String value '{}' not found in config '{}'", key, config_name
            )))
    }

    pub fn get_bool_value(&self, config_name: &str, key: &str) -> Result<bool, WebPipeError> {
        let config_json = self.resolve_config_as_json(config_name)?;
        
        config_json
            .get(key)
            .and_then(|v| v.as_bool())
            .ok_or_else(|| WebPipeError::ConfigError(format!(
                "Boolean value '{}' not found in config '{}'", key, config_name
            )))
    }

    pub fn get_number_value(&self, config_name: &str, key: &str) -> Result<i64, WebPipeError> {
        let config_json = self.resolve_config_as_json(config_name)?;
        
        config_json
            .get(key)
            .and_then(|v| v.as_i64())
            .ok_or_else(|| WebPipeError::ConfigError(format!(
                "Number value '{}' not found in config '{}'", key, config_name
            )))
    }
}

/// Initialize the global ConfigManager once for the process
pub fn init_global(configs: Vec<Config>) {
    let _ = GLOBAL_CONFIG.set(ConfigManager::new(configs));
}

/// Access the global ConfigManager (must be initialized at startup)
pub fn global() -> &'static ConfigManager {
    GLOBAL_CONFIG.get().expect("Global ConfigManager not initialized")
}