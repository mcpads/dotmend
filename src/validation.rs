use crate::art::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LocationRole {
    Observed,
    Expected,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckLocation {
    pub region: Rect,
    pub role: LocationRole,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub id: String,
    pub status: CheckStatus,
    pub expected: Value,
    pub actual: Value,
    pub locations: Vec<CheckLocation>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Validation {
    pub art_id: String,
    pub status: CheckStatus,
    pub checks: Vec<CheckResult>,
    pub pixels_hash: String,
    pub target_hash: String,
    pub implementation: String,
    pub scope: String,
    pub visual_approval: String,
    pub runtime: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bounds {
    region: Rect,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bottom {
    y: u32,
}

enum Geometry {
    Bounds(Rect),
    Bottom(u32),
    Unsupported,
}
fn parameters(target: &Target, position: usize) -> ArtResult<Geometry> {
    let req = &target.requirements[position];
    let parsed = (|| -> ArtResult<Geometry> {
        match req.kind.as_str() {
            "opaque_bounds" => {
                let value: Bounds = serde_json::from_value(req.parameters.clone())
                    .map_err(|e| invalid(e.to_string()))?;
                value
                    .region
                    .check_within(target.bounds())
                    .map_err(|e| invalid(e.message))?;
                Ok(Geometry::Bounds(value.region))
            }
            "opaque_bottom" => {
                let value: Bottom = serde_json::from_value(req.parameters.clone())
                    .map_err(|e| invalid(e.to_string()))?;
                if value.y >= target.height {
                    return Err(invalid("Baseline is outside the canvas"));
                }
                Ok(Geometry::Bottom(value.y))
            }
            _ => Ok(Geometry::Unsupported),
        }
    })();
    parsed.map_err(|error| error.detail(json!({"requirement_index":position,"requirement_id":req.id,"field":format!("requirements[{position}].parameters"),"parameters":req.parameters})))
}
pub fn check_parameters(target: &Target) -> ArtResult<()> {
    for position in 0..target.requirements.len() {
        parameters(target, position)?;
    }
    Ok(())
}
pub fn include_point(bounds: &mut Option<Rect>, x: u32, y: u32) {
    *bounds = Some(match *bounds {
        None => Rect {
            x,
            y,
            width: 1,
            height: 1,
        },
        Some(r) => {
            let left = r.x.min(x);
            let top = r.y.min(y);
            Rect {
                x: left,
                y: top,
                width: (r.x + r.width).max(x + 1) - left,
                height: (r.y + r.height).max(y + 1) - top,
            }
        }
    });
}
pub fn inspect_constraints(art: &Art) -> ArtResult<Validation> {
    check_parameters(&art.target)?;
    let valid = art.check();
    let mut checks = vec![CheckResult {
        id: "pixel_data".into(),
        status: if valid.is_ok() {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        expected: json!("An index array matching the target constraints"),
        actual: match &valid {
            Ok(()) => json!("valid"),
            Err(e) => json!(e),
        },
        locations: vec![],
    }];
    for (position, req) in art.target.requirements.iter().enumerate() {
        let geometry = parameters(&art.target, position)?;
        let mut check = CheckResult {
            id: req.id.clone(),
            status: CheckStatus::Unknown,
            expected: json!(req),
            actual: json!("unsupported requirement kind"),
            locations: vec![],
        };
        if valid.is_err() {
            check.actual = json!("invalid pixel data");
            checks.push(check);
            continue;
        }
        if matches!(geometry, Geometry::Unsupported) {
            checks.push(check);
            continue;
        }
        let mut bounds = None;
        let mut count = 0usize;
        let mut outside = 0usize;
        let mut strips = [None; 4];
        let mut bottom_row = None;
        for y in 0..art.target.height {
            for x in 0..art.target.width {
                if Some(art.indices[y as usize][x as usize]) == art.target.transparent_index {
                    continue;
                }
                count += 1;
                include_point(&mut bounds, x, y);
                if bottom_row.is_some_and(|r: Rect| r.y != y) {
                    bottom_row = None;
                }
                include_point(&mut bottom_row, x, y);
                if let Geometry::Bounds(r) = geometry {
                    let strip = if y < r.y {
                        Some(0)
                    } else if y >= r.y + r.height {
                        Some(1)
                    } else if x < r.x {
                        Some(2)
                    } else if x >= r.x + r.width {
                        Some(3)
                    } else {
                        None
                    };
                    if let Some(i) = strip {
                        outside += 1;
                        include_point(&mut strips[i], x, y);
                    }
                }
            }
        }
        check.expected = req.parameters.clone();
        let passed = match geometry {
            Geometry::Bounds(_) => {
                check.actual =
                    json!({"opaque_count":count,"bounds":bounds,"outside_count":outside});
                check.locations = strips
                    .into_iter()
                    .flatten()
                    .map(|region| CheckLocation {
                        region,
                        role: LocationRole::Observed,
                    })
                    .collect();
                outside == 0
            }
            Geometry::Bottom(y) => {
                check.actual = json!({"opaque_count":count,"y":bottom_row.map(|r|r.y)});
                let pass = bottom_row.is_some_and(|r| r.y == y);
                if !pass {
                    if let Some(region) = bottom_row {
                        check.locations.push(CheckLocation {
                            region,
                            role: LocationRole::Observed,
                        });
                    }
                    check.locations.push(CheckLocation {
                        region: Rect {
                            x: 0,
                            y,
                            width: art.target.width,
                            height: 1,
                        },
                        role: LocationRole::Expected,
                    });
                }
                pass
            }
            Geometry::Unsupported => unreachable!(),
        };
        check.status = if passed {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        };
        checks.push(check);
    }
    let status = if checks.iter().any(|c| c.status == CheckStatus::Fail) {
        CheckStatus::Fail
    } else if checks.iter().any(|c| c.status == CheckStatus::Unknown) {
        CheckStatus::Unknown
    } else {
        CheckStatus::Pass
    };
    Ok(Validation {
        art_id: art.id()?,
        status,
        checks,
        pixels_hash: hash_json(&art.indices)?,
        target_hash: hash_json(&art.target)?,
        implementation: implementation_id(),
        scope: "provided_constraints".into(),
        visual_approval: "unassessed".into(),
        runtime: "unobserved".into(),
    })
}
