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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manager() -> ConfigManager {
        let cfg = Config {
            name: "app".to_string(),
            properties: vec![
                crate::ast::ConfigProperty { key: "host".to_string(), value: crate::ast::ConfigValue::String("localhost".to_string()) },
                crate::ast::ConfigProperty { key: "enabled".to_string(), value: crate::ast::ConfigValue::Boolean(true) },
                crate::ast::ConfigProperty { key: "port".to_string(), value: crate::ast::ConfigValue::Number(8080) },
                crate::ast::ConfigProperty { key: "apiKey".to_string(), value: crate::ast::ConfigValue::EnvVar { var: "MISSING_ENV".to_string(), default: Some("fallback".to_string()) } },
            ],
        };
        ConfigManager::new(vec![cfg])
    }

    #[test]
    fn resolve_values_and_getters() {
        let cm = sample_manager();
        assert_eq!(cm.get_string_value("app", "host").unwrap(), "localhost");
        assert_eq!(cm.get_bool_value("app", "enabled").unwrap(), true);
        assert_eq!(cm.get_number_value("app", "port").unwrap(), 8080);

        // Env with default falls back
        let as_json = cm.resolve_config_as_json("app").unwrap();
        assert_eq!(as_json["apiKey"], serde_json::json!("fallback"));
    }

    #[test]
    fn env_without_default_errors_and_missing_config_errors() {
        let cm = sample_manager();
        // Missing env and no default -> error
        let err = cm.resolve_config_value(&crate::ast::ConfigValue::EnvVar { var: "NO_SUCH_ENV".to_string(), default: None }).unwrap_err();
        match err { WebPipeError::ConfigError(_) => {}, other => panic!("unexpected error: {:?}", other) }

        // Missing config name -> error
        let err2 = cm.resolve_config_as_json("nope").unwrap_err();
        match err2 { WebPipeError::ConfigError(_) => {}, other => panic!("unexpected error: {:?}", other) }
    }
}