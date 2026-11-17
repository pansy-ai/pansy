//! Routing groups: several models an agent picks as one (`group/daily`),
//! which the gateway spreads requests over. Saved in `groups.json`.

use serde::{Deserialize, Serialize};

/// How a group picks the member that answers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Routing {
    /// The first member until it cannot answer, then the next.
    #[default]
    Order,
    /// The next member on each new turn.
    Rotate,
    /// The member used least since the gateway started first.
    Usage,
}

impl Routing {
    pub const ALL: [Routing; 3] = [Routing::Order, Routing::Rotate, Routing::Usage];

    pub fn as_str(self) -> &'static str {
        match self {
            Routing::Order => "order",
            Routing::Rotate => "rotate",
            Routing::Usage => "usage",
        }
    }

    pub fn parse(s: &str) -> Option<Routing> {
        match s {
            "order" | "failover" => Some(Routing::Order),
            "rotate" | "round-robin" => Some(Routing::Rotate),
            "usage" | "least-used" => Some(Routing::Usage),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// Its id in model names (`daily` in `group/daily`).
    pub id: String,
    pub name: String,
    /// Members in order: `provider/model`, or `group/<id>` for a group
    /// inside this one.
    pub members: Vec<String>,
    #[serde(default)]
    pub routing: Routing,
    /// A middleware's routing that orders the members (`<plugin>/<name>`),
    /// over `routing`; kept as saved while no plugin serves it, and the
    /// members are then tried in their listed order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

/// A group's members with the groups inside it opened, in order, each once;
/// a group that contains itself stops there.
pub fn flatten(groups: &[Group], id: &str) -> Vec<String> {
    fn walk(groups: &[Group], id: &str, seen: &mut Vec<String>, out: &mut Vec<String>) {
        if seen.iter().any(|s| s == id) {
            return;
        }
        seen.push(id.to_owned());
        let Some(group) = groups.iter().find(|g| g.id == id) else {
            return;
        };
        for member in &group.members {
            match member.strip_prefix("group/") {
                Some(inner) => walk(groups, inner, seen, out),
                None => {
                    if !out.contains(member) {
                        out.push(member.clone());
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(groups, id, &mut Vec::new(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_nested_groups_once() {
        let g = |id: &str, members: &[&str]| Group {
            id: id.into(),
            name: id.into(),
            members: members.iter().map(|m| m.to_string()).collect(),
            routing: Routing::Order,
            strategy: None,
        };
        let groups = vec![
            g("a", &["x/1", "group/b", "y/2"]),
            g("b", &["z/3", "x/1", "group/a"]),
        ];
        assert_eq!(flatten(&groups, "a"), ["x/1", "z/3", "y/2"]);
    }
}
