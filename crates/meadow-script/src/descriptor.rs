//! WorldPainter script descriptors: `// script.name=...`, `// script.param.<name>.<prop>=...`
//! comment lines at the top of a script, used to build a parameter form.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamType {
    Integer,
    Percentage,
    Float,
    String,
    Boolean,
    File,
}

#[derive(Debug, Clone)]
pub struct ScriptParam {
    pub name: String,
    pub kind: ParamType,
    pub description: Option<String>,
    pub display_name: Option<String>,
    pub default: Option<String>,
    pub optional: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

impl ScriptParam {
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ScriptDescriptor {
    pub name: Option<String>,
    pub description: Option<String>,
    pub params: Vec<ScriptParam>,
    pub hide_cmd_line_params: bool,
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n").replace("\\t", "\t")
}

pub fn parse_descriptor(source: &str) -> ScriptDescriptor {
    let mut d = ScriptDescriptor::default();
    for line in source.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Some(c) = t.strip_prefix("//") else { break };
        let c = c.trim();
        let Some(rest) = c.strip_prefix("script.") else { continue };
        let Some((key, value)) = rest.split_once('=') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "name" => d.name = Some(unescape(value)),
            "description" => d.description = Some(unescape(value)),
            "hideCmdLineParams" => d.hide_cmd_line_params = value.eq_ignore_ascii_case("true"),
            k => {
                let Some(p) = k.strip_prefix("param.") else { continue };
                let Some((pname, prop)) = p.rsplit_once('.') else { continue };
                let idx = match d.params.iter().position(|x| x.name == pname) {
                    Some(i) => i,
                    None => {
                        d.params.push(ScriptParam {
                            name: pname.to_owned(),
                            kind: ParamType::String,
                            description: None,
                            display_name: None,
                            default: None,
                            optional: false,
                            min: None,
                            max: None,
                        });
                        d.params.len() - 1
                    }
                };
                let p = &mut d.params[idx];
                match prop {
                    "type" => {
                        p.kind = match value.to_lowercase().as_str() {
                            "integer" | "int" => ParamType::Integer,
                            "percentage" => ParamType::Percentage,
                            "float" | "double" => ParamType::Float,
                            "boolean" | "bool" => ParamType::Boolean,
                            "file" => ParamType::File,
                            _ => ParamType::String,
                        }
                    }
                    "description" => p.description = Some(unescape(value)),
                    "displayName" => p.display_name = Some(value.to_owned()),
                    "default" => p.default = Some(value.to_owned()),
                    "optional" => p.optional = value.eq_ignore_ascii_case("true"),
                    "min" => p.min = value.parse().ok(),
                    "max" => p.max = value.parse().ok(),
                    _ => {}
                }
            }
        }
    }
    d
}
