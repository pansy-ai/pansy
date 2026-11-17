//! The vendors pansy knows, as magpie lists them: where each answers, in
//! which protocols, its logo, its plans or regions (each at endpoints of
//! its own), and the models it offers when it has no list to ask for.

use crate::provider::{Endpoint, Protocol, Quirks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A model's own maker.
    Vendor,
    /// A relay: one key for many vendors' models (OpenRouter, SiliconFlow).
    Relay,
    /// A server on this computer, or the user's own (Ollama, GPUStack).
    Local,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Vendor => "vendor",
            Kind::Relay => "relay",
            Kind::Local => "local",
        }
    }
}

/// One of a preset's plans or regions: the endpoints picking it moves the
/// provider to. The first is the preset's own.
#[derive(Debug, Clone)]
pub struct Region {
    pub id: &'static str,
    pub name: &'static str,
    pub urls: &'static [(Protocol, &'static str)],
    /// Where its keys are made, when that is not the preset's page.
    pub key_url: &'static str,
    /// Its docs, when they are not the preset's.
    pub website: &'static str,
    /// A plan's models, for a plan whose endpoints list none.
    pub models: &'static [&'static str],
    /// It lists its models although the preset has none to ask for.
    pub lists: bool,
}

#[derive(Debug, Clone)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// The add sheet's name for it, when `name` is long.
    pub short: &'static str,
    /// Its logo in the window (`desktop/ui/logos`).
    pub icon: &'static str,
    pub kind: Kind,
    /// Base URLs by protocol; a preset with regions is at its first one's.
    pub urls: &'static [(Protocol, &'static str)],
    pub website: &'static str,
    /// Where the user gets a key.
    pub key_url: &'static str,
    /// One line under the name.
    pub note: &'static str,
    /// The key field's placeholder, when it says more than "paste an API key".
    pub key_hint: &'static str,
    /// Models offered until the vendor's list is fetched, or for good when
    /// it has none.
    pub models: &'static [&'static str],
    /// Its model list is at `GET {base}/models`.
    pub lists_models: bool,
    /// A key is optional: a local server, a relay's free models.
    pub keyless: bool,
    pub regions: &'static [Region],
    /// What the regions choose between: "Region" or "Plan".
    pub region_label: &'static str,
    /// Request headers the vendor documents, offered in the editor.
    pub header_hints: &'static [&'static str],
    /// For a plan sold on a key that buys much else: what its models' ids
    /// start with; the rest of the list is left out.
    pub only: &'static str,
    /// For a vendor at the user's own resource (Azure OpenAI): an example
    /// of its address, which the user gives in place of `urls`.
    pub endpoint: &'static str,
    pub endpoint_hint: &'static str,
    pub openai: bool,
    pub reasoning_content: bool,
}

impl Preset {
    /// The region at `id`, or none.
    pub fn region(&self, id: &str) -> Option<&'static Region> {
        self.regions.iter().find(|r| r.id == id)
    }

    /// The base URLs at `region`, else the preset's own.
    pub fn urls_at(&self, region: Option<&Region>) -> &'static [(Protocol, &'static str)] {
        match region.or(self.regions.first()) {
            Some(r) => r.urls,
            None => self.urls,
        }
    }

    pub fn endpoints(&self) -> Vec<Endpoint> {
        self.endpoints_at(None)
    }

    pub fn endpoints_at(&self, region: Option<&Region>) -> Vec<Endpoint> {
        self.urls_at(region)
            .iter()
            .map(|(protocol, url)| Endpoint {
                protocol: *protocol,
                base_url: (*url).to_owned(),
            })
            .collect()
    }

    /// The host it answers at (`api.deepseek.com`), the endpoint's example
    /// for one at the user's own.
    pub fn host(&self) -> String {
        let url = self
            .urls_at(None)
            .iter()
            .find(|(p, _)| *p == Protocol::Chat)
            .or(self.urls_at(None).first())
            .map_or(self.endpoint, |(_, url)| *url);
        crate::provider::host_of(url)
    }

    pub fn key_url_at(&self, region: Option<&Region>) -> &'static str {
        match region {
            Some(r) if !r.key_url.is_empty() => r.key_url,
            _ => self.key_url,
        }
    }

    pub fn website_at(&self, region: Option<&Region>) -> &'static str {
        match region {
            Some(r) if !r.website.is_empty() => r.website,
            _ => self.website,
        }
    }

    /// The models it gives at `region`: the plan's, else its own.
    pub fn models_at(&self, region: Option<&Region>) -> &'static [&'static str] {
        match region.or(self.regions.first()) {
            Some(r) if !r.models.is_empty() => r.models,
            _ => self.models,
        }
    }

    /// Whether its list is asked for at `region`.
    pub fn lists_at(&self, region: Option<&Region>) -> bool {
        region.or(self.regions.first()).is_some_and(|r| r.lists) || self.lists_models
    }

    pub fn local(&self) -> bool {
        self.kind == Kind::Local
    }

    pub fn quirks(&self) -> Quirks {
        Quirks {
            openai: self.openai,
            reasoning_content: self.reasoning_content,
            anthropic_bearer: false,
        }
    }
}

const fn preset(
    id: &'static str,
    name: &'static str,
    icon: &'static str,
    kind: Kind,
    urls: &'static [(Protocol, &'static str)],
    website: &'static str,
    key_url: &'static str,
) -> Preset {
    Preset {
        id,
        name,
        short: "",
        icon,
        kind,
        urls,
        website,
        key_url,
        note: "",
        key_hint: "",
        models: &[],
        lists_models: true,
        keyless: false,
        regions: &[],
        region_label: "Region",
        header_hints: &[],
        only: "",
        endpoint: "",
        endpoint_hint: "",
        openai: false,
        reasoning_content: false,
    }
}

const fn region(
    id: &'static str,
    name: &'static str,
    urls: &'static [(Protocol, &'static str)],
) -> Region {
    Region {
        id,
        name,
        urls,
        key_url: "",
        website: "",
        models: &[],
        lists: false,
    }
}

use Kind::*;
use Protocol::*;

/// Amazon Bedrock's runtime in each region, named by its code: Claude on
/// Anthropic's Messages, the rest on chat completions and Responses.
macro_rules! bedrock_regions {
    ($($r:literal),*) => {
        &[$(region($r, $r, &[
            (Chat, concat!("https://bedrock-runtime.", $r, ".amazonaws.com/openai/v1")),
            (Responses, concat!("https://bedrock-runtime.", $r, ".amazonaws.com/openai/v1")),
            (Anthropic, concat!("https://bedrock-runtime.", $r, ".amazonaws.com/anthropic")),
        ])),*]
    };
}

/// Bedrock's Mantle endpoint in each region that serves it.
macro_rules! mantle_regions {
    ($($r:literal),*) => {
        &[$(region($r, $r, &[
            (Chat, concat!("https://bedrock-mantle.", $r, ".api.aws/v1")),
            (Responses, concat!("https://bedrock-mantle.", $r, ".api.aws/v1")),
            (Anthropic, concat!("https://bedrock-mantle.", $r, ".api.aws/anthropic")),
        ])),*]
    };
}

/// Qwen Cloud's pay-as-you-go regions, each at its own host.
macro_rules! qwencloud_regions {
    ($($r:literal),*) => {
        &[$(region($r, $r, &[
            (Chat, concat!("https://", $r, ".maas.qwencloudapi.com/compatible-mode/v1")),
            (Anthropic, concat!("https://", $r, ".maas.qwencloudapi.com/apps/anthropic")),
        ])),*]
    };
}

/// The text models of Volcengine Ark's Coding Plan and Agent Plan, whose
/// endpoints list Ark's whole catalog instead.
const ARK_PLAN_MODELS: &[&str] = &[
    "ark-code-latest",
    "doubao-seed-evolving",
    "doubao-seed-2.1-pro",
    "doubao-seed-2.1-lite",
    "doubao-seed-2.0-mini",
    "minimax-m3",
    "glm-5.3",
    "glm-5.3-flash",
    "deepseek-v4.1-flash",
    "deepseek-v4.1-flash-economy",
    "deepseek-v4-flash",
    "deepseek-v4-pro",
    "kimi-k2.8-preview",
    "kimi-k3",
    "kimi-k3.1-pro",
];

/// The text models of Alibaba's Token Plan, on the Qwen AI platform's and
/// on Bailian's alike.
const ALIBABA_PLAN_MODELS: &[&str] = &[
    "auto",
    "qwen3.8-max",
    "qwen3.8-flash",
    "qwen3.7-max",
    "qwen3.7-plus",
    "qwen3.6-flash",
    "deepseek-v4.1-flash",
    "deepseek-v4-pro",
    "deepseek-v4-pro-0813",
    "deepseek-v4-flash-0731",
    "glm-5.3",
    "glm-5.2",
];

/// Every preset, in the order the add sheet lists them.
pub static PRESETS: &[Preset] = &[
    // The models' own makers.
    Preset {
        header_hints: &["anthropic-workspace-id"],
        ..preset(
            "anthropic",
            "Anthropic",
            "claude-color",
            Vendor,
            &[(Anthropic, "https://api.anthropic.com")],
            "https://console.anthropic.com",
            "https://console.anthropic.com/settings/keys",
        )
    },
    Preset {
        openai: true,
        ..preset(
            "openai",
            "OpenAI",
            "openai",
            Vendor,
            &[
                (Chat, "https://api.openai.com/v1"),
                (Responses, "https://api.openai.com/v1"),
            ],
            "https://platform.openai.com",
            "https://platform.openai.com/api-keys",
        )
    },
    // Gemini's own API too, which Gemini CLI's requests pass through to.
    Preset {
        note: "Gemini Developer API",
        ..preset(
            "google",
            "Google Gemini",
            "gemini-color",
            Vendor,
            &[
                (
                    Chat,
                    "https://generativelanguage.googleapis.com/v1beta/openai",
                ),
                (Gemini, "https://generativelanguage.googleapis.com"),
            ],
            "https://aistudio.google.com",
            "https://aistudio.google.com/apikey",
        )
    },
    Preset {
        reasoning_content: true,
        ..preset(
            "deepseek",
            "DeepSeek",
            "deepseek-color",
            Vendor,
            &[
                (Chat, "https://api.deepseek.com/v1"),
                (Responses, "https://api.deepseek.com/v1"),
                (Anthropic, "https://api.deepseek.com/anthropic"),
            ],
            "https://platform.deepseek.com",
            "https://platform.deepseek.com/api_keys",
        )
    },
    preset(
        "xai",
        "xAI",
        "xai",
        Vendor,
        &[
            (Chat, "https://api.x.ai/v1"),
            (Responses, "https://api.x.ai/v1"),
            (Anthropic, "https://api.x.ai"),
        ],
        "https://console.x.ai",
        "https://console.x.ai",
    ),
    // A vendor's China endpoints are a preset of their own, `<id>-cn`: the
    // add sheet shows the two as one, picked between in the add dialog.
    Preset {
        reasoning_content: true,
        ..preset(
            "moonshot",
            "Kimi",
            "kimi",
            Vendor,
            &[
                (Chat, "https://api.moonshot.ai/v1"),
                (Anthropic, "https://api.moonshot.ai/anthropic"),
            ],
            "https://platform.kimi.ai",
            "https://platform.kimi.ai/console/api-keys",
        )
    },
    Preset {
        reasoning_content: true,
        ..preset(
            "moonshot-cn",
            "Kimi (China)",
            "kimi",
            Vendor,
            &[
                (Chat, "https://api.moonshot.cn/v1"),
                (Anthropic, "https://api.moonshot.cn/anthropic"),
            ],
            "https://platform.kimi.com",
            "https://platform.kimi.com/console/api-keys",
        )
    },
    // A Kimi Code membership's own endpoints, keyed at its console.
    Preset {
        note: "Membership",
        models: &["kimi-for-coding"],
        reasoning_content: true,
        ..preset(
            "kimi-code",
            "Kimi Code",
            "kimi",
            Vendor,
            &[
                (Chat, "https://api.kimi.ai/coding/v1"),
                (Anthropic, "https://api.kimi.ai/coding"),
            ],
            "https://www.kimi.ai/code",
            "https://www.kimi.ai/code/console",
        )
    },
    Preset {
        note: "Membership",
        models: &["kimi-for-coding"],
        reasoning_content: true,
        ..preset(
            "kimi-code-cn",
            "Kimi Code (China)",
            "kimi",
            Vendor,
            &[
                (Chat, "https://api.kimi.com/coding/v1"),
                (Anthropic, "https://api.kimi.com/coding"),
            ],
            "https://www.kimi.com/code",
            "https://www.kimi.com/code/console",
        )
    },
    // A GLM Coding Plan is served at its own endpoints: a plan's key sent
    // to the pay-as-you-go ones is told it has no balance.
    Preset {
        region_label: "Plan",
        regions: &[
            region(
                "api",
                "Pay as you go",
                &[
                    (Chat, "https://open.bigmodel.cn/api/paas/v4"),
                    (Anthropic, "https://open.bigmodel.cn/api/anthropic"),
                ],
            ),
            region(
                "coding",
                "Coding Plan",
                &[
                    (Chat, "https://open.bigmodel.cn/api/coding/paas/v4"),
                    (Responses, "https://open.bigmodel.cn/api/v1"),
                    (Anthropic, "https://open.bigmodel.cn/api/anthropic"),
                ],
            ),
        ],
        reasoning_content: true,
        ..preset(
            "zhipu",
            "Zhipu GLM",
            "zhipu-color",
            Vendor,
            &[],
            "https://open.bigmodel.cn",
            "https://open.bigmodel.cn/usercenter/proj-mgmt/apikeys",
        )
    },
    Preset {
        region_label: "Plan",
        regions: &[
            region(
                "api",
                "Pay as you go",
                &[
                    (Chat, "https://api.z.ai/api/paas/v4"),
                    (Anthropic, "https://api.z.ai/api/anthropic"),
                ],
            ),
            region(
                "coding",
                "Coding Plan",
                &[
                    (Chat, "https://api.z.ai/api/coding/paas/v4"),
                    (Responses, "https://api.z.ai/api/v1"),
                    (Anthropic, "https://api.z.ai/api/anthropic"),
                ],
            ),
        ],
        reasoning_content: true,
        ..preset(
            "zai",
            "Z.ai",
            "zai",
            Vendor,
            &[],
            "https://z.ai",
            "https://z.ai/manage-apikey/apikey-list",
        )
    },
    preset(
        "minimax",
        "MiniMax",
        "minimax-color",
        Vendor,
        &[
            (Chat, "https://api.minimax.io/v1"),
            (Anthropic, "https://api.minimax.io/anthropic"),
        ],
        "https://platform.minimax.io",
        "https://platform.minimax.io/user-center/basic-information/interface-key",
    ),
    preset(
        "minimax-cn",
        "MiniMax (China)",
        "minimax-color",
        Vendor,
        &[
            (Chat, "https://api.minimaxi.com/v1"),
            (Anthropic, "https://api.minimaxi.com/anthropic"),
        ],
        "https://platform.minimaxi.com",
        "https://platform.minimaxi.com/user-center/basic-information/interface-key",
    ),
    // A Step Plan is served at its own endpoints, its key refused at the
    // pay-as-you-go ones.
    Preset {
        region_label: "Plan",
        regions: &[
            region(
                "plan",
                "Step Plan",
                &[
                    (Chat, "https://api.stepfun.ai/step_plan/v1"),
                    (Anthropic, "https://api.stepfun.ai/step_plan"),
                ],
            ),
            region(
                "api",
                "Pay as you go",
                &[
                    (Chat, "https://api.stepfun.ai/v1"),
                    (Anthropic, "https://api.stepfun.ai"),
                ],
            ),
        ],
        ..preset(
            "stepfun",
            "StepFun",
            "stepfun-color",
            Vendor,
            &[],
            "https://platform.stepfun.ai",
            "https://platform.stepfun.ai/interface-key",
        )
    },
    Preset {
        region_label: "Plan",
        regions: &[
            region(
                "plan",
                "Step Plan",
                &[
                    (Chat, "https://api.stepfun.com/step_plan/v1"),
                    (Anthropic, "https://api.stepfun.com/step_plan"),
                ],
            ),
            region(
                "api",
                "Pay as you go",
                &[
                    (Chat, "https://api.stepfun.com/v1"),
                    (Anthropic, "https://api.stepfun.com"),
                ],
            ),
        ],
        ..preset(
            "stepfun-cn",
            "StepFun (China)",
            "stepfun-color",
            Vendor,
            &[],
            "https://platform.stepfun.com",
            "https://platform.stepfun.com/interface-key",
        )
    },
    // A Token Plan's key is spent at its region's own host, pay as you go
    // at api.xiaomimimo.com.
    Preset {
        note: "Token Plan · pay as you go",
        regions: &[
            region(
                "plan-cn",
                "Plan · China",
                &[
                    (Chat, "https://token-plan-cn.xiaomimimo.com/v1"),
                    (Responses, "https://token-plan-cn.xiaomimimo.com/v1"),
                    (Anthropic, "https://token-plan-cn.xiaomimimo.com/anthropic"),
                ],
            ),
            region(
                "plan-sgp",
                "Plan · Singapore",
                &[
                    (Chat, "https://token-plan-sgp.xiaomimimo.com/v1"),
                    (Responses, "https://token-plan-sgp.xiaomimimo.com/v1"),
                    (Anthropic, "https://token-plan-sgp.xiaomimimo.com/anthropic"),
                ],
            ),
            region(
                "plan-ams",
                "Plan · Europe",
                &[
                    (Chat, "https://token-plan-ams.xiaomimimo.com/v1"),
                    (Responses, "https://token-plan-ams.xiaomimimo.com/v1"),
                    (Anthropic, "https://token-plan-ams.xiaomimimo.com/anthropic"),
                ],
            ),
            region(
                "api",
                "Pay as you go",
                &[
                    (Chat, "https://api.xiaomimimo.com/v1"),
                    (Responses, "https://api.xiaomimimo.com/v1"),
                    (Anthropic, "https://api.xiaomimimo.com/anthropic"),
                ],
            ),
        ],
        ..preset(
            "xiaomi",
            "Xiaomi MiMo",
            "mimocode",
            Vendor,
            &[],
            "https://platform.xiaomimimo.com",
            "https://platform.xiaomimimo.com/token-plan",
        )
    },
    // Meituan's LongCat: chat completions and Responses under /openai/v1,
    // Anthropic messages under /anthropic.
    preset(
        "longcat",
        "LongCat",
        "longcat",
        Vendor,
        &[
            (Chat, "https://api.longcat.chat/openai/v1"),
            (Responses, "https://api.longcat.chat/openai/v1"),
            (Anthropic, "https://api.longcat.chat/anthropic"),
        ],
        "https://longcat.chat/platform/docs/zh/api-docs",
        "https://longcat.chat/platform/api_keys",
    ),
    // A personal and an enterprise Token Plan, each at its own endpoints
    // on a key of its own, which list no models; pay as you go lists them.
    Preset {
        note: "Token Plan · pay as you go",
        region_label: "Plan",
        regions: &[
            region(
                "personal",
                "Token Plan Personal",
                &[
                    (Chat, "https://qianfan.baidubce.com/v2/tokenplan/personal"),
                    (
                        Responses,
                        "https://qianfan.baidubce.com/v2/tokenplan/personal",
                    ),
                    (
                        Anthropic,
                        "https://qianfan.baidubce.com/anthropic/tokenplan/personal",
                    ),
                ],
            ),
            region(
                "team",
                "Token Plan Enterprise",
                &[
                    (Chat, "https://qianfan.baidubce.com/v2/tokenplan/team"),
                    (Responses, "https://qianfan.baidubce.com/v2/tokenplan/team"),
                    (
                        Anthropic,
                        "https://qianfan.baidubce.com/anthropic/tokenplan/team",
                    ),
                ],
            ),
            Region {
                lists: true,
                key_url: "https://console.bce.baidu.com/iam/#/iam/apikey/list",
                ..region(
                    "api",
                    "Pay as you go",
                    &[
                        (Chat, "https://qianfan.baidubce.com/v2"),
                        (Responses, "https://qianfan.baidubce.com/v2"),
                        (Anthropic, "https://qianfan.baidubce.com/anthropic"),
                    ],
                )
            },
        ],
        lists_models: false,
        models: &[
            "qianfan-code-latest",
            "deepseek-v4.1-flash",
            "deepseek-v4-pro",
            "deepseek-v4-pro-0813",
            "deepseek-v4-flash",
            "deepseek-v4-flash-0731",
            "deepseek-v3.2",
            "glm-5.3",
            "glm-5.3-flash",
            "glm-5.2",
            "glm-5.1",
            "glm-5",
        ],
        ..preset(
            "baidu-qianfan",
            "Baidu Qianfan",
            "baiducloud-color",
            Vendor,
            &[],
            "https://cloud.baidu.com/doc/qianfan/s/Dmrabu8b6",
            "https://console.bce.baidu.com/qianfan/resource/token-plan",
        )
    },
    // Tencent Cloud's Token Plan and TokenHub's pay as you go, in China
    // and abroad.
    Preset {
        note: "Token Plan · pay as you go",
        regions: &[
            Region {
                website: "https://cloud.tencent.com/document/product/1823/130060",
                key_url: "https://console.cloud.tencent.com/tokenhub/tokenplan",
                models: &[
                    "tc-code-latest",
                    "auto",
                    "glm-5.3",
                    "glm-5.3-flash",
                    "glm-5.2",
                    "glm-5.1",
                    "glm-5",
                    "kimi-k3",
                    "kimi-k2.7-code",
                    "kimi-k2.7-code-highspeed",
                    "kimi-k2.6",
                    "deepseek-v4-pro-202606",
                    "deepseek-v4-flash-202605",
                    "deepseek-v4.1-flash",
                    "deepseek-v4-pro",
                    "deepseek-v4-pro-0813",
                    "deepseek-v4-flash-0731",
                    "minimax-m3",
                    "minimax-m2.7",
                    "hy4-preview",
                    "hy3",
                    "mimo-v2.6-pro",
                    "mimo-v2.6-flash",
                ],
                ..region(
                    "plan",
                    "Plan · China",
                    &[
                        (Chat, "https://tokenhub.tencentmaas.com/plan/v3"),
                        (Responses, "https://tokenhub.tencentmaas.com/plan/v3"),
                        (Anthropic, "https://tokenhub.tencentmaas.com/plan/anthropic"),
                    ],
                )
            },
            Region {
                website: "https://cloud.tencent.com/document/product/1823/130078",
                key_url: "https://console.cloud.tencent.com/tokenhub/apikey",
                ..region(
                    "cn",
                    "Pay as you go · China",
                    &[
                        (Chat, "https://tokenhub.tencentmaas.com/v1"),
                        (Responses, "https://tokenhub.tencentmaas.com/v1"),
                        (Anthropic, "https://tokenhub.tencentmaas.com"),
                    ],
                )
            },
            Region {
                website: "https://www.tencentcloud.com/document/product/1300/78939",
                key_url: "https://console.tencentcloud.com/tokenhub/apikey",
                ..region(
                    "intl",
                    "Pay as you go · Global",
                    &[
                        (Chat, "https://tokenhub-intl.tencentmaas.com/v1"),
                        (Responses, "https://tokenhub-intl.tencentmaas.com/v1"),
                        (Anthropic, "https://tokenhub-intl.tencentmaas.com"),
                    ],
                )
            },
            Region {
                website: "https://cloud.tencent.com/document/product/1823/130659",
                key_url: "https://console.tencentcloud.com/tokenhub/tokenplan",
                models: &[
                    "auto",
                    "hy4-preview",
                    "glm-5.3",
                    "glm-5.3-flash",
                    "glm-5.2",
                    "kimi-k3",
                    "kimi-k2.7-code",
                    "kimi-k2.7-code-highspeed",
                    "minimax-m3",
                    "deepseek-v4.1-flash",
                    "deepseek-v4-pro",
                    "deepseek-v4-pro-0813",
                    "deepseek-v4-flash-0731",
                    "deepseek-v4-pro-202606",
                    "deepseek-v4-flash-202605",
                    "mimo-v2.6-pro",
                    "mimo-v2.6-flash",
                ],
                ..region(
                    "enterprise-intl",
                    "Enterprise plan · Global",
                    &[
                        (Chat, "https://tokenhub-intl.tencentmaas.com/plan/v3"),
                        (Responses, "https://tokenhub-intl.tencentmaas.com/plan/v3"),
                        (
                            Anthropic,
                            "https://tokenhub-intl.tencentmaas.com/plan/anthropic",
                        ),
                    ],
                )
            },
        ],
        ..preset(
            "tencent-cloud",
            "Tencent Cloud",
            "tencentcloud-color",
            Vendor,
            &[],
            "https://cloud.tencent.com/document/product/1823/130060",
            "https://console.cloud.tencent.com/tokenhub/tokenplan",
        )
    },
    // Huawei Cloud MaaS's Token Plan, its quota spent only at the plan's
    // own endpoints under /plan.
    Preset {
        short: "Huawei Cloud",
        note: "Token Plan · 西南-贵阳一",
        region_label: "Plan",
        regions: &[
            region(
                "plan",
                "Token Plan",
                &[
                    (Chat, "https://api.modelarts-maas.com/plan/v2"),
                    (Anthropic, "https://api.modelarts-maas.com/plan/anthropic"),
                ],
            ),
            region(
                "api",
                "Pay as you go",
                &[
                    (Chat, "https://api.modelarts-maas.com/openai/v1"),
                    (Anthropic, "https://api.modelarts-maas.com/anthropic"),
                ],
            ),
        ],
        models: &[
            "glm-5.3",
            "glm-5.1",
            "kimi-k2.6",
            "deepseek-v4.1-flash",
            "deepseek-v4-flash",
        ],
        ..preset(
            "huaweicloud",
            "Huawei Cloud MaaS",
            "huaweicloud-color",
            Vendor,
            &[],
            "https://support.huaweicloud.com/Token-plan-maas/tokenplan-maas-0001.html",
            "https://console.huaweicloud.com/modelarts/?#/model-studio/authmanage",
        )
    },
    // Volcengine Ark: a Coding Plan's quota is spent only at /api/coding,
    // an Agent Plan's only at /api/plan, each with a key of its own; pay as
    // you go is /api/v3, the only one whose list is the account's.
    Preset {
        note: "火山方舟 · Coding / Agent Plan",
        region_label: "Plan",
        regions: &[
            Region {
                models: ARK_PLAN_MODELS,
                ..region(
                    "coding",
                    "Coding Plan",
                    &[
                        (Chat, "https://ark.cn-beijing.volces.com/api/coding/v3"),
                        (Responses, "https://ark.cn-beijing.volces.com/api/coding/v3"),
                        (Anthropic, "https://ark.cn-beijing.volces.com/api/coding"),
                    ],
                )
            },
            Region {
                models: ARK_PLAN_MODELS,
                ..region(
                    "agent",
                    "Agent Plan",
                    &[
                        (Chat, "https://ark.cn-beijing.volces.com/api/plan/v3"),
                        (Responses, "https://ark.cn-beijing.volces.com/api/plan/v3"),
                        (Anthropic, "https://ark.cn-beijing.volces.com/api/plan"),
                    ],
                )
            },
            Region {
                lists: true,
                ..region(
                    "api",
                    "Pay as you go",
                    &[
                        (Chat, "https://ark.cn-beijing.volces.com/api/v3"),
                        (Responses, "https://ark.cn-beijing.volces.com/api/v3"),
                    ],
                )
            },
        ],
        lists_models: false,
        ..preset(
            "volcengine",
            "Volcengine Ark",
            "volcengine-color",
            Vendor,
            &[],
            "https://www.volcengine.com/docs/82379/1925114",
            "https://ark.volcengine.com/region:cn-beijing/apikey",
        )
    },
    Preset {
        note: "DashScope · intl",
        ..preset(
            "qwen",
            "Qwen",
            "qwen-color",
            Vendor,
            &[
                (
                    Chat,
                    "https://dashscope-intl.aliyuncs.com/compatible-mode/v1",
                ),
                (
                    Anthropic,
                    "https://dashscope-intl.aliyuncs.com/apps/anthropic",
                ),
            ],
            "https://modelstudio.console.alibabacloud.com",
            "https://modelstudio.console.alibabacloud.com/?tab=playground#/api-key",
        )
    },
    Preset {
        note: "DashScope · China",
        ..preset(
            "qwen-cn",
            "Qwen (China)",
            "qwen-color",
            Vendor,
            &[
                (Chat, "https://dashscope.aliyuncs.com/compatible-mode/v1"),
                (Anthropic, "https://dashscope.aliyuncs.com/apps/anthropic"),
            ],
            "https://bailian.console.aliyun.com",
            "https://bailian.console.aliyun.com/?tab=model#/api-key",
        )
    },
    // Alibaba's Token Plan, sold on the Qwen AI platform, on Bailian, on
    // Alibaba Cloud abroad and on Qwen Cloud, each on keys that only its
    // own host takes.
    Preset {
        short: "Qwen Plan",
        note: "Qwen AI platform · subscription",
        models: ALIBABA_PLAN_MODELS,
        ..preset(
            "qwen-token-plan",
            "Qwen Token Plan",
            "qwen-color",
            Vendor,
            &[
                (
                    Chat,
                    "https://token-plan.maas.qianwenaiapi.com/compatible-mode/v1",
                ),
                (
                    Anthropic,
                    "https://token-plan.maas.qianwenaiapi.com/apps/anthropic",
                ),
            ],
            "https://platform.qianwenai.com/docs/token-plan/overview",
            "https://platform.qianwenai.com/home/analytics/token-plan/individual",
        )
    },
    Preset {
        short: "Bailian Plan",
        note: "Alibaba Cloud Bailian · subscription",
        models: ALIBABA_PLAN_MODELS,
        ..preset(
            "bailian-token-plan",
            "Bailian Token Plan",
            "bailian-color",
            Vendor,
            &[
                (
                    Chat,
                    "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1",
                ),
                (
                    Anthropic,
                    "https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic",
                ),
            ],
            "https://help.aliyun.com/zh/model-studio/token-plan-overview",
            "https://bailian.console.aliyun.com/cn-beijing/subscription/token-plan/personal",
        )
    },
    Preset {
        short: "Alibaba Plan",
        note: "Alibaba Cloud Model Studio · subscription",
        models: ALIBABA_PLAN_MODELS,
        ..preset(
            "alibaba-token-plan",
            "Alibaba Cloud Token Plan",
            "bailian-color",
            Vendor,
            &[
                (
                    Chat,
                    "https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
                ),
                (
                    Anthropic,
                    "https://token-plan.ap-southeast-1.maas.aliyuncs.com/apps/anthropic",
                ),
            ],
            "https://www.alibabacloud.com/help/en/model-studio/token-plan-personal-quick-start",
            "https://modelstudio.console.alibabacloud.com/ap-southeast-1/?tab=globalset#/efm/token_plan",
        )
    },
    Preset {
        short: "Qwen Cloud Plan",
        note: "Qwen Cloud · subscription",
        models: &[
            "auto",
            "qwen3.8-max",
            "qwen3.8-flash",
            "qwen3.7-max",
            "qwen3.7-plus",
            "qwen3.6-flash",
            "deepseek-v4.1-flash",
            "deepseek-v4.1-flash-1017",
            "deepseek-v4-pro",
            "deepseek-v4-pro-0813",
            "deepseek-v4-flash-0731",
            "glm-5.3",
            "glm-5.2",
        ],
        ..preset(
            "qwencloud-token-plan",
            "Qwen Cloud Token Plan",
            "qwen-color",
            Vendor,
            &[
                (
                    Chat,
                    "https://token-plan.maas.qwencloudapi.com/compatible-mode/v1",
                ),
                (
                    Anthropic,
                    "https://token-plan.maas.qwencloudapi.com/apps/anthropic",
                ),
            ],
            "https://docs.qwencloud.com/token-plan/overview",
            "https://home.qwencloud.com/analytics/token-plan/individual",
        )
    },
    Preset {
        note: "Qwen Cloud · pay as you go",
        regions: qwencloud_regions!(
            "ap-southeast-1",
            "us-east-1",
            "eu-central-1",
            "ap-northeast-1",
            "cn-hongkong",
            "cn-beijing"
        ),
        ..preset(
            "qwencloud",
            "Qwen Cloud",
            "qwen-color",
            Vendor,
            &[],
            "https://docs.qwencloud.com/developer-guides/getting-started/regions",
            "https://home.qwencloud.com/api-keys",
        )
    },
    preset(
        "mistral",
        "Mistral",
        "mistral-color",
        Vendor,
        &[(Chat, "https://api.mistral.ai/v1")],
        "https://console.mistral.ai",
        "https://console.mistral.ai/api-keys",
    ),
    preset(
        "groq",
        "Groq",
        "groq",
        Vendor,
        &[
            (Chat, "https://api.groq.com/openai/v1"),
            (Responses, "https://api.groq.com/openai/v1"),
        ],
        "https://console.groq.com",
        "https://console.groq.com/keys",
    ),
    // With a Bedrock API key: the runtime has no list to ask, so its
    // models are given, Claude and GPT as their global inference profiles.
    Preset {
        note: "Bedrock API key",
        regions: bedrock_regions!(
            "us-east-1",
            "us-east-2",
            "us-west-2",
            "eu-central-1",
            "eu-west-1",
            "eu-west-3",
            "ap-northeast-1",
            "ap-southeast-1",
            "ap-southeast-2",
            "ap-south-1"
        ),
        lists_models: false,
        models: &[
            "global.anthropic.claude-opus-5-5",
            "global.anthropic.claude-sonnet-5-5",
            "global.anthropic.claude-sonnet-5",
            "global.anthropic.claude-opus-5",
            "global.anthropic.claude-fable-5-2",
            "global.anthropic.claude-fable-5-1",
            "global.anthropic.claude-opus-4-8",
            "global.anthropic.claude-opus-4-7",
            "global.anthropic.claude-haiku-4-5-20251001-v1:0",
            "global.openai.gpt-6-astra",
            "global.openai.gpt-6-sol",
            "global.openai.gpt-6-luna",
            "openai.gpt-oss-120b-1:0",
            "openai.gpt-oss-20b-1:0",
            "qwen.qwen3-coder-480b-a35b-v1:0",
            "deepseek.v3.2",
            "moonshotai.kimi-k2.5",
            "zai.glm-5",
            "minimax.minimax-m2.5",
        ],
        ..preset(
            "bedrock",
            "Amazon Bedrock",
            "bedrock-color",
            Vendor,
            &[],
            "https://aws.amazon.com/bedrock/",
            "https://console.aws.amazon.com/bedrock/home#/api-keys",
        )
    },
    // Bedrock's Mantle endpoint, with the same key: it lists its models.
    Preset {
        short: "Bedrock Mantle",
        note: "Bedrock API key",
        regions: mantle_regions!(
            "us-east-1",
            "us-east-2",
            "us-west-2",
            "eu-central-1",
            "eu-central-2",
            "eu-north-1",
            "eu-south-1",
            "eu-west-1",
            "eu-west-2",
            "ap-northeast-1",
            "ap-northeast-2",
            "ap-south-1",
            "ap-southeast-1",
            "ap-southeast-2",
            "ap-southeast-3",
            "ap-southeast-4",
            "sa-east-1"
        ),
        ..preset(
            "bedrock-mantle",
            "Amazon Bedrock (Mantle)",
            "bedrock-color",
            Vendor,
            &[],
            "https://docs.aws.amazon.com/bedrock/latest/userguide/endpoints.html",
            "https://console.aws.amazon.com/bedrock/home#/api-keys",
        )
    },
    // Azure OpenAI at the user's own resource, on its v1 API: the key in
    // api-key, the deployments' names as the model ids.
    Preset {
        note: "your resource's endpoint and key",
        endpoint: "https://<resource>.openai.azure.com",
        endpoint_hint: "Your resource's endpoint, from Keys and Endpoint in the Azure portal. Once added, pansy asks the resource for its deployments with this key, over its v1 API; the deployments' names are the model ids.",
        openai: true,
        ..preset(
            AZURE,
            "Azure OpenAI",
            "azure-color",
            Vendor,
            &[],
            "https://ai.azure.com",
            "https://portal.azure.com/#view/Microsoft_Azure_ProjectOxford/CognitiveServicesHub/~/OpenAI",
        )
    },
    Preset {
        note: "cloud models, with an API key",
        ..preset(
            "ollama-cloud",
            "Ollama Cloud",
            "ollama",
            Vendor,
            &[
                (Chat, "https://ollama.com/v1"),
                (Anthropic, "https://ollama.com"),
            ],
            "https://docs.ollama.com/cloud",
            "https://ollama.com/settings/keys",
        )
    },
    Preset {
        note: "Nous Research · Hermes and other models",
        ..preset(
            "nous",
            "Nous Portal",
            "nousresearch",
            Vendor,
            &[(Chat, "https://inference-api.nousresearch.com/v1")],
            "https://portal.nousresearch.com",
            "https://portal.nousresearch.com",
        )
    },
    // Relays: one key, many vendors.
    Preset {
        header_hints: &["HTTP-Referer", "X-OpenRouter-Title"],
        ..preset(
            "openrouter",
            "OpenRouter",
            "openrouter",
            Relay,
            &[
                (Chat, "https://openrouter.ai/api/v1"),
                (Anthropic, "https://openrouter.ai/api"),
            ],
            "https://openrouter.ai",
            "https://openrouter.ai/keys",
        )
    },
    preset(
        "vercel",
        "Vercel AI Gateway",
        "vercel",
        Relay,
        &[
            (Chat, "https://ai-gateway.vercel.sh/v1"),
            (Responses, "https://ai-gateway.vercel.sh/v1"),
            (Anthropic, "https://ai-gateway.vercel.sh"),
        ],
        "https://vercel.com/docs/ai-gateway",
        "https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai-gateway%2Fapi-keys",
    ),
    preset(
        "gmicloud",
        "GMI Cloud",
        "gmicloud",
        Relay,
        &[(Chat, "https://api.gmi-serving.com/v1")],
        "https://www.gmicloud.ai",
        "https://console.gmicloud.ai",
    ),
    Preset {
        note: "open coding models, $10/month",
        ..preset(
            "opencode-go",
            "OpenCode Go",
            "opencode",
            Relay,
            &[
                (Chat, "https://opencode.ai/inference/go/openai/v1"),
                (Responses, "https://opencode.ai/inference/go/openai/v1"),
                (Anthropic, "https://opencode.ai/inference/go/anthropic"),
            ],
            "https://opencode.ai/docs/go",
            "https://opencode.ai/auth",
        )
    },
    // Cline's plan for open models: the Cline API lists only its paid
    // models, so the plan's are given.
    Preset {
        note: "open coding models, $9.99/month",
        only: "cline-pass/",
        models: &[
            "cline-pass/glm-5.3",
            "cline-pass/glm-5.3-flash",
            "cline-pass/kimi-k3",
            "cline-pass/deepseek-v4-pro",
            "cline-pass/deepseek-v4.1-flash",
            "cline-pass/mimo-v2.5",
            "cline-pass/mimo-v2.5-pro",
            "cline-pass/minimax-m3",
            "cline-pass/muse-spark-1.3-contributor",
            "cline-pass/qwen3.8-max",
            "cline-pass/qwen3.7-max",
            "cline-pass/qwen3.7-plus",
        ],
        ..preset(
            "clinepass",
            "ClinePass",
            "cline",
            Relay,
            &[(Chat, "https://api.cline.bot/api/v1")],
            "https://docs.cline.bot/getting-started/clinepass",
            "https://app.cline.bot",
        )
    },
    Preset {
        keyless: true,
        key_hint: "optional: free models need no key",
        ..preset(
            "opencode-zen",
            "OpenCode Zen",
            "opencode",
            Relay,
            &[
                (Chat, "https://opencode.ai/zen/v1"),
                (Responses, "https://opencode.ai/zen/v1"),
                (Anthropic, "https://opencode.ai/zen"),
            ],
            "https://opencode.ai/docs/zen",
            "https://opencode.ai/auth",
        )
    },
    // Kilo Code's gateway: its free models are served with no key.
    Preset {
        keyless: true,
        note: "free models with no key",
        key_hint: "optional: free models need no key",
        ..preset(
            "kilo",
            "Kilo Gateway",
            "kilo",
            Relay,
            &[(Chat, "https://api.kilo.ai/api/openrouter")],
            "https://kilo.ai/docs/gateway",
            "https://app.kilo.ai",
        )
    },
    preset(
        "commandcode",
        "Command Code",
        "commandcode",
        Relay,
        &[
            (Chat, "https://api.commandcode.ai/provider/v1"),
            (Responses, "https://api.commandcode.ai/provider/v1"),
            (Anthropic, "https://api.commandcode.ai/provider"),
        ],
        "https://commandcode.ai/docs/provider",
        "https://commandcode.ai/settings/keys",
    ),
    preset(
        "together",
        "Together AI",
        "together-color",
        Relay,
        &[(Chat, "https://api.together.xyz/v1")],
        "https://api.together.ai",
        "https://api.together.ai/settings/api-keys",
    ),
    preset(
        "fireworks",
        "Fireworks",
        "fireworks-color",
        Relay,
        &[(Chat, "https://api.fireworks.ai/inference/v1")],
        "https://fireworks.ai",
        "https://app.fireworks.ai/settings/users/api-keys",
    ),
    // Fireworks Nexus: Pro and Team take only `auto`, Nexus Auto, which
    // reaches Claude and GPT with the user's own keys in the two headers.
    Preset {
        short: "Nexus",
        note: "Nexus Auto · Pro, Team, Enterprise",
        lists_models: false,
        models: &["auto"],
        header_hints: &["x-anthropic-api-key", "x-openai-api-key"],
        ..preset(
            "fireworks-nexus",
            "Fireworks Nexus",
            "fireworks-color",
            Relay,
            &[
                (Chat, "https://api.fireworks.ai/inference/v1"),
                (Responses, "https://api.fireworks.ai/inference/v1"),
                (Anthropic, "https://api.fireworks.ai/inference"),
            ],
            "https://docs.fireworks.ai/nexus",
            "https://nexus.fireworks.ai/settings/api-keys",
        )
    },
    // The international site is another account, with keys of its own.
    Preset {
        regions: &[
            region("cn", "China", &[(Chat, "https://api.siliconflow.cn/v1")]),
            Region {
                website: "https://cloud.siliconflow.com",
                key_url: "https://cloud.siliconflow.com/account/ak",
                ..region(
                    "intl",
                    "Global",
                    &[(Chat, "https://api.siliconflow.com/v1")],
                )
            },
        ],
        ..preset(
            "siliconflow",
            "SiliconFlow",
            "siliconcloud-color",
            Relay,
            &[],
            "https://cloud.siliconflow.cn",
            "https://cloud.siliconflow.cn/account/ak",
        )
    },
    preset(
        "nvidia",
        "NVIDIA NIM",
        "nvidia-color",
        Relay,
        &[(Chat, "https://integrate.api.nvidia.com/v1")],
        "https://build.nvidia.com",
        "https://build.nvidia.com/settings/api-keys",
    ),
    Preset {
        note: "魔搭 · API-Inference",
        ..preset(
            "modelscope",
            "ModelScope",
            "modelscope-color",
            Relay,
            &[
                (Chat, "https://api-inference.modelscope.cn/v1"),
                (Responses, "https://api-inference.modelscope.cn/v1"),
            ],
            "https://modelscope.cn/docs/model-service/API-Inference/intro",
            "https://modelscope.cn/my/myaccesstoken",
        )
    },
    preset(
        "aihubmix",
        "AiHubMix",
        "aihubmix-color",
        Relay,
        &[
            (Chat, "https://aihubmix.com/v1"),
            (Anthropic, "https://aihubmix.com"),
        ],
        "https://aihubmix.com",
        "https://console.aihubmix.com/token",
    ),
    preset(
        "pipellm",
        "PipeLLM",
        "pipellm-color",
        Relay,
        &[
            (Chat, "https://api.pipellm.ai/openai/v1"),
            (Responses, "https://api.pipellm.ai/v1"),
            (Anthropic, "https://api.pipellm.ai"),
        ],
        "https://www.pipellm.ai",
        "https://console.pipellm.ai",
    ),
    preset(
        "302ai",
        "302.AI",
        "ai302-color",
        Relay,
        &[
            (Chat, "https://api.302.ai/v1"),
            (Anthropic, "https://api.302.ai"),
        ],
        "https://302.ai",
        "https://302.ai/api-keys/list",
    ),
    preset(
        "cherryin",
        "CherryIN",
        "cherryin-color",
        Relay,
        &[
            (Chat, "https://open.cherryin.ai/v1"),
            (Responses, "https://open.cherryin.ai/v1"),
            (Anthropic, "https://open.cherryin.ai"),
        ],
        "https://open.cherryin.ai",
        "https://open.cherryin.ai/console/token",
    ),
    Preset {
        regions: &[
            region(
                "auto",
                "Auto",
                &[
                    (Chat, "https://app.yylx.io/v1"),
                    (Anthropic, "https://app.yylx.io"),
                ],
            ),
            region(
                "global",
                "Global",
                &[
                    (Chat, "https://global.yylx.io/v1"),
                    (Anthropic, "https://global.yylx.io"),
                ],
            ),
            region(
                "cn",
                "China Mainland",
                &[
                    (Chat, "https://cn.yylx.io/v1"),
                    (Anthropic, "https://cn.yylx.io"),
                ],
            ),
        ],
        ..preset(
            "yylx",
            "鱼鱼连线",
            "yylx",
            Relay,
            &[],
            "https://yylx.io",
            "https://app.yylx.io/keys",
        )
    },
    // Servers on this computer, or the user's own: their address is the
    // user's to give, each URL keeping its path.
    Preset {
        keyless: true,
        note: "your local models",
        ..preset(
            "ollama",
            "Ollama",
            "ollama",
            Local,
            &[
                (Chat, "http://localhost:11434/v1"),
                (Anthropic, "http://localhost:11434"),
            ],
            "https://ollama.com",
            "",
        )
    },
    Preset {
        keyless: true,
        note: "local server, port 1234 by default",
        ..preset(
            "lmstudio",
            "LM Studio",
            "lmstudio",
            Local,
            &[(Chat, "http://localhost:1234/v1")],
            "https://lmstudio.ai",
            "",
        )
    },
    Preset {
        keyless: true,
        note: "local server, port 8000 by default",
        ..preset(
            "omlx",
            "oMLX",
            "omlx",
            Local,
            &[
                (Chat, "http://localhost:8000/v1"),
                (Responses, "http://localhost:8000/v1"),
                (Anthropic, "http://localhost:8000"),
            ],
            "https://omlx.ai",
            "",
        )
    },
    Preset {
        note: "your own server and API key",
        ..preset(
            "gpustack",
            "GPUStack",
            "gpustack",
            Local,
            &[(Chat, "http://localhost/v1")],
            "https://github.com/gpustack/gpustack",
            "",
        )
    },
    Preset {
        keyless: true,
        note: "local server, port 11234 by default",
        ..preset(
            "mlx-serve",
            "MLX-Serve",
            "mlx-serve",
            Local,
            &[
                (Chat, "http://localhost:11234/v1"),
                (Responses, "http://localhost:11234/v1"),
                (Anthropic, "http://localhost:11234"),
            ],
            "https://github.com/ddalcu/mlx-serve",
            "",
        )
    },
];

/// Azure OpenAI's preset id.
pub const AZURE: &str = "azure";

/// Ids presets had before pansy took magpie's, each the preset since and
/// the plan or region it is.
const ALIASES: &[(&str, &str, &str)] = &[
    ("gemini", "google", ""),
    ("moonshot-intl", "moonshot", ""),
    ("kimi-coding", "kimi-code-cn", ""),
    ("minimax-intl", "minimax", ""),
    ("qwen-intl", "qwen", ""),
    ("mimo", "xiaomi", "api"),
    ("baidu", "baidu-qianfan", "api"),
    ("hunyuan", "tencent-cloud", "cn"),
    ("huawei", "huaweicloud", "api"),
    ("siliconflow-intl", "siliconflow", "intl"),
];

/// The preset of an id, or of an id it had before.
pub fn find(id: &str) -> Option<&'static Preset> {
    let id = ALIASES
        .iter()
        .find(|(old, _, _)| *old == id)
        .map_or(id, |(_, now, _)| now);
    PRESETS.iter().find(|p| p.id == id)
}

/// The region an id from before stands for (`siliconflow-intl`: Global).
pub fn alias_region(id: &str) -> Option<&'static Region> {
    let (_, now, region) = ALIASES.iter().find(|(old, _, _)| *old == id)?;
    find(now)?.region(region)
}

/// A vendor's China preset (`moonshot-cn`) beside its global one.
pub fn china_of(preset: &Preset) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == format!("{}-cn", preset.id))
}

/// The global preset a China one (`moonshot-cn`) is beside.
pub fn global_of(preset: &Preset) -> Option<&'static Preset> {
    let id = preset.id.strip_suffix("-cn")?;
    PRESETS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_urls_absolute() {
        let mut ids: Vec<&str> = PRESETS.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PRESETS.len());
        for p in PRESETS {
            assert!(!p.icon.is_empty(), "{}", p.id);
            assert_eq!(crate::provider::slug(p.id), p.id);
            // A preset is at its urls, at its first region's, or at the
            // user's own endpoint; never two of them.
            let ways = [
                !p.urls.is_empty(),
                !p.regions.is_empty(),
                !p.endpoint.is_empty(),
            ];
            assert_eq!(ways.iter().filter(|w| **w).count(), 1, "{}", p.id);
            let urls = p.urls.iter().chain(p.regions.iter().flat_map(|r| r.urls));
            for (_, url) in urls {
                assert!(url.starts_with("https://") || p.local(), "{}: {url}", p.id);
                assert!(!url.ends_with('/'), "{}: {url}", p.id);
            }
            for r in p.regions {
                assert!(r.urls.iter().any(|(p, _)| *p == Chat), "{}/{}", p.id, r.id);
            }
        }
        for (old, now, region) in ALIASES {
            assert!(PRESETS.iter().all(|p| p.id != *old), "{old}");
            assert!(find(now).is_some(), "{now}");
            assert!(region.is_empty() || alias_region(old).is_some(), "{old}");
        }
    }

    #[test]
    fn regions_move_the_endpoints_and_the_models() {
        let ark = find("volcengine").unwrap();
        let coding = ark.endpoints();
        assert_eq!(
            coding[0].base_url,
            "https://ark.cn-beijing.volces.com/api/coding/v3"
        );
        assert_eq!(ark.models_at(None)[0], "ark-code-latest");
        assert!(!ark.lists_at(None));
        let api = ark.region("api");
        assert!(ark.lists_at(api));
        assert!(ark.models_at(api).is_empty());
        let global = find("siliconflow").unwrap().region("intl");
        assert_eq!(
            find("siliconflow").unwrap().key_url_at(global),
            "https://cloud.siliconflow.com/account/ak"
        );
        assert_eq!(find("bedrock").unwrap().regions.len(), 10);
        assert_eq!(
            find("bedrock").unwrap().endpoints()[2].base_url,
            "https://bedrock-runtime.us-east-1.amazonaws.com/anthropic"
        );
        assert_eq!(find("gemini").unwrap().id, "google");
        assert_eq!(
            china_of(find("moonshot").unwrap()).unwrap().id,
            "moonshot-cn"
        );
        assert_eq!(global_of(find("qwen-cn").unwrap()).unwrap().id, "qwen");
        assert!(global_of(find("zai").unwrap()).is_none());
    }
}
