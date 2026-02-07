use rquickjs::{Context, Function, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHookProviderInfo {
    pub id: String,
    pub name: String,
}

/// onRequest 脚本上下文（只读）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestHookContext {
    pub app: String,
    pub method: String,
    pub path: String,
    pub endpoint: String,
    pub url: String,
    pub provider: RequestHookProviderInfo,
    #[serde(rename = "incomingHeaders")]
    pub incoming_headers: HashMap<String, String>,
}

/// onRequest 可修改的请求视图（最终将发往上游）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookRequest {
    pub headers: HashMap<String, String>,
    pub queries: HashMap<String, String>,
    pub body: Value,
}

/// onResponse 可修改的响应视图（最终返回给客户端）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookResponse {
    pub code: u16,
    pub headers: HashMap<String, String>,
    pub body: Value,
}

fn stringify_header_value(value: &axum::http::HeaderValue) -> String {
    value
        .to_str()
        .map(str::to_string)
        .unwrap_or_else(|_| String::from_utf8_lossy(value.as_bytes()).to_string())
}

pub(crate) fn build_header_string_map(headers: &axum::http::HeaderMap) -> HashMap<String, String> {
    let mut output: HashMap<String, String> = HashMap::new();
    for (key, value) in headers {
        let name = key.as_str().to_ascii_lowercase();
        let value_str = stringify_header_value(value);
        output
            .entry(name)
            .and_modify(|existing| {
                existing.push_str(",");
                existing.push_str(&value_str);
            })
            .or_insert(value_str);
    }
    output
}

pub(crate) fn build_query_string_map_from_url(url: &str) -> HashMap<String, String> {
    let mut output = HashMap::new();
    let Ok(parsed) = url::Url::parse(url) else {
        return output;
    };
    for (key, value) in parsed.query_pairs() {
        output.insert(key.into_owned(), value.into_owned());
    }
    output
}

pub(crate) fn apply_query_string_map_to_url(
    url: &str,
    queries: &HashMap<String, String>,
) -> Result<String, String> {
    let mut parsed = url::Url::parse(url).map_err(|e| format!("解析 URL 失败: {e}"))?;
    {
        let mut pairs = parsed.query_pairs_mut();
        pairs.clear();
        for (key, value) in queries {
            pairs.append_pair(key, value);
        }
    }
    Ok(parsed.to_string())
}

pub(crate) fn execute_on_request_script(
    script_code: &str,
    context: &RequestHookContext,
    request: &HookRequest,
) -> Result<Option<HookRequest>, String> {
    let runtime = Runtime::new().map_err(|e| format!("创建 JS 运行时失败: {e}"))?;
    let js_context = Context::full(&runtime).map_err(|e| format!("创建 JS 上下文失败: {e}"))?;

    js_context.with(|ctx| {
        let config: rquickjs::Object = ctx
            .eval(script_code)
            .map_err(|e| format!("解析脚本失败（脚本必须 eval 成一个对象）: {e}"))?;

        let on_request: Option<Function> = config.get("onRequest").ok();
        let Some(on_request) = on_request else {
            return Ok(None);
        };

        let context_json =
            serde_json::to_string(context).map_err(|e| format!("序列化 context 失败: {e}"))?;
        let request_json =
            serde_json::to_string(request).map_err(|e| format!("序列化 request 失败: {e}"))?;

        let context_js: rquickjs::Value = ctx
            .json_parse(context_json.as_str())
            .map_err(|e| format!("解析 context JSON 失败: {e}"))?;
        let request_js: rquickjs::Value = ctx
            .json_parse(request_json.as_str())
            .map_err(|e| format!("解析 request JSON 失败: {e}"))?;

        let result_js: rquickjs::Value = on_request
            .call((context_js, request_js))
            .map_err(|e| format!("执行 onRequest 失败: {e}"))?;

        let result_json = ctx
            .json_stringify(result_js)
            .map_err(|e| format!("序列化 onRequest 返回值失败: {e}"))?;

        let Some(result_json) = result_json else {
            // undefined: 视为放行（不修改）
            return Ok(None);
        };

        let result_str: String = result_json
            .get()
            .map_err(|e| format!("获取 onRequest 返回值字符串失败: {e}"))?;

        if result_str.trim() == "null" {
            // null: 视为放行（不修改）
            return Ok(None);
        }

        let result_value: Value = serde_json::from_str(&result_str)
            .map_err(|e| format!("解析 onRequest 返回值 JSON 失败: {e}"))?;

        let merged = merge_hook_request(&result_value, request)?;
        Ok(Some(merged))
    })
}

pub(crate) fn execute_on_response_script(
    script_code: &str,
    context: &RequestHookContext,
    response: &HookResponse,
) -> Result<Option<HookResponse>, String> {
    let runtime = Runtime::new().map_err(|e| format!("创建 JS 运行时失败: {e}"))?;
    let js_context = Context::full(&runtime).map_err(|e| format!("创建 JS 上下文失败: {e}"))?;

    js_context.with(|ctx| {
        let config: rquickjs::Object = ctx
            .eval(script_code)
            .map_err(|e| format!("解析脚本失败（脚本必须 eval 成一个对象）: {e}"))?;

        let on_response: Option<Function> = config.get("onResponse").ok();
        let Some(on_response) = on_response else {
            return Ok(None);
        };

        let context_json =
            serde_json::to_string(context).map_err(|e| format!("序列化 context 失败: {e}"))?;
        let response_json =
            serde_json::to_string(response).map_err(|e| format!("序列化 response 失败: {e}"))?;

        let context_js: rquickjs::Value = ctx
            .json_parse(context_json.as_str())
            .map_err(|e| format!("解析 context JSON 失败: {e}"))?;
        let response_js: rquickjs::Value = ctx
            .json_parse(response_json.as_str())
            .map_err(|e| format!("解析 response JSON 失败: {e}"))?;

        let result_js: rquickjs::Value = on_response
            .call((context_js, response_js))
            .map_err(|e| format!("执行 onResponse 失败: {e}"))?;

        let result_json = ctx
            .json_stringify(result_js)
            .map_err(|e| format!("序列化 onResponse 返回值失败: {e}"))?;

        let Some(result_json) = result_json else {
            return Ok(None);
        };

        let result_str: String = result_json
            .get()
            .map_err(|e| format!("获取 onResponse 返回值字符串失败: {e}"))?;

        if result_str.trim() == "null" {
            return Ok(None);
        }

        let result_value: Value = serde_json::from_str(&result_str)
            .map_err(|e| format!("解析 onResponse 返回值 JSON 失败: {e}"))?;

        let merged = merge_hook_response(&result_value, response)?;
        Ok(Some(merged))
    })
}

fn merge_hook_request(result: &Value, original: &HookRequest) -> Result<HookRequest, String> {
    let obj = result
        .as_object()
        .ok_or_else(|| "onRequest 必须返回一个对象（通常是 request）".to_string())?;

    let headers = if let Some(headers_val) = obj.get("headers") {
        let headers_obj = headers_val.as_object().ok_or_else(|| {
            "onRequest 返回值中的 request.headers 必须是对象（Record<string,string>）".to_string()
        })?;

        let mut out: HashMap<String, String> = HashMap::new();
        for (k, v) in headers_obj {
            let Some(v_str) = v.as_str() else {
                return Err(format!(
                    "request.headers[\"{k}\"] 必须是字符串（当前类型: {}）",
                    v
                ));
            };
            out.insert(k.to_ascii_lowercase(), v_str.to_string());
        }
        out
    } else {
        original.headers.clone()
    };

    let queries = if let Some(queries_val) = obj.get("queries") {
        let queries_obj = queries_val.as_object().ok_or_else(|| {
            "onRequest 返回值中的 request.queries 必须是对象（Record<string,string>）".to_string()
        })?;

        let mut out: HashMap<String, String> = HashMap::new();
        for (k, v) in queries_obj {
            let Some(v_str) = v.as_str() else {
                return Err(format!(
                    "request.queries[\"{k}\"] 必须是字符串（当前类型: {}）",
                    v
                ));
            };
            out.insert(k.to_string(), v_str.to_string());
        }
        out
    } else {
        original.queries.clone()
    };

    let body = obj
        .get("body")
        .cloned()
        .unwrap_or_else(|| original.body.clone());

    Ok(HookRequest {
        headers,
        queries,
        body,
    })
}

fn merge_hook_response(result: &Value, original: &HookResponse) -> Result<HookResponse, String> {
    let obj = result
        .as_object()
        .ok_or_else(|| "onResponse 必须返回一个对象（通常是 response）".to_string())?;

    let code = if let Some(code_val) = obj.get("code") {
        let Some(code_num) = code_val.as_u64() else {
            return Err(format!(
                "response.code 必须是数字（当前类型: {}）",
                code_val
            ));
        };
        u16::try_from(code_num).map_err(|_| format!("response.code 超出有效范围: {code_num}"))?
    } else {
        original.code
    };

    let headers = if let Some(headers_val) = obj.get("headers") {
        let headers_obj = headers_val.as_object().ok_or_else(|| {
            "onResponse 返回值中的 response.headers 必须是对象（Record<string,string>）".to_string()
        })?;

        let mut out: HashMap<String, String> = HashMap::new();
        for (k, v) in headers_obj {
            let Some(v_str) = v.as_str() else {
                return Err(format!(
                    "response.headers[\"{k}\"] 必须是字符串（当前类型: {}）",
                    v
                ));
            };
            out.insert(k.to_ascii_lowercase(), v_str.to_string());
        }
        out
    } else {
        original.headers.clone()
    };

    let body = obj
        .get("body")
        .cloned()
        .unwrap_or_else(|| original.body.clone());

    Ok(HookResponse {
        code,
        headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn on_request_can_delete_header() {
        let script = r#"
({
  onRequest: function(context, request) {
    delete request.headers["x-codex-turn-metadata"];
    return request;
  }
})
"#;
        let ctx = RequestHookContext {
            app: "codex".to_string(),
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            endpoint: "/v1/responses".to_string(),
            url: "https://api.openai.com/v1/responses".to_string(),
            provider: RequestHookProviderInfo {
                id: "p1".to_string(),
                name: "Provider".to_string(),
            },
            incoming_headers: HashMap::new(),
        };
        let mut headers = HashMap::new();
        headers.insert(
            "x-codex-turn-metadata".to_string(),
            r#"{"workspaces":{"/Users/xx/项目/思考":{}}}"#.to_string(),
        );
        headers.insert("user-agent".to_string(), "ua".to_string());
        let req = HookRequest {
            headers,
            queries: HashMap::new(),
            body: json!({"model":"gpt-4.1"}),
        };
        let out = execute_on_request_script(script, &ctx, &req)
            .unwrap()
            .unwrap();
        assert!(!out.headers.contains_key("x-codex-turn-metadata"));
        assert_eq!(out.headers.get("user-agent").unwrap(), "ua");
    }

    #[test]
    fn on_request_undefined_is_passthrough() {
        let script = r#"
({
  onRequest: function(context, request) {
    // no return
  }
})
"#;
        let ctx = RequestHookContext {
            app: "codex".to_string(),
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            endpoint: "/v1/responses".to_string(),
            url: "https://api.openai.com/v1/responses".to_string(),
            provider: RequestHookProviderInfo {
                id: "p1".to_string(),
                name: "Provider".to_string(),
            },
            incoming_headers: HashMap::new(),
        };
        let mut headers = HashMap::new();
        headers.insert("x-test".to_string(), "1".to_string());
        let req = HookRequest {
            headers: headers.clone(),
            queries: HashMap::new(),
            body: json!({"ok":true}),
        };
        let out = execute_on_request_script(script, &ctx, &req).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn on_request_can_rewrite_queries() {
        let script = r#"
({
  onRequest: function(context, request) {
    request.queries.foo = "bar";
    delete request.queries.remove_me;
    return request;
  }
})
"#;
        let ctx = RequestHookContext {
            app: "codex".to_string(),
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            endpoint: "/v1/responses".to_string(),
            url: "https://api.openai.com/v1/responses?remove_me=1".to_string(),
            provider: RequestHookProviderInfo {
                id: "p1".to_string(),
                name: "Provider".to_string(),
            },
            incoming_headers: HashMap::new(),
        };
        let req = HookRequest {
            headers: HashMap::new(),
            queries: HashMap::from([
                ("remove_me".to_string(), "1".to_string()),
                ("keep".to_string(), "yes".to_string()),
            ]),
            body: json!({"ok":true}),
        };
        let out = execute_on_request_script(script, &ctx, &req)
            .unwrap()
            .unwrap();
        assert_eq!(out.queries.get("foo").unwrap(), "bar");
        assert_eq!(out.queries.get("keep").unwrap(), "yes");
        assert!(!out.queries.contains_key("remove_me"));
    }

    #[test]
    fn on_response_can_modify_status_headers_and_body() {
        let script = r#"
({
  onResponse: function(context, response) {
    response.code = 404;
    response.headers["x-hook-response"] = "ok";
    response.body = { ok: false };
    return response;
  }
})
"#;
        let ctx = RequestHookContext {
            app: "codex".to_string(),
            method: "POST".to_string(),
            path: "/v1/responses".to_string(),
            endpoint: "/v1/responses".to_string(),
            url: "https://api.openai.com/v1/responses".to_string(),
            provider: RequestHookProviderInfo {
                id: "p1".to_string(),
                name: "Provider".to_string(),
            },
            incoming_headers: HashMap::new(),
        };
        let resp = HookResponse {
            code: 200,
            headers: HashMap::from([("content-type".to_string(), "application/json".to_string())]),
            body: json!({"ok":true}),
        };
        let out = execute_on_response_script(script, &ctx, &resp)
            .unwrap()
            .unwrap();
        assert_eq!(out.code, 404);
        assert_eq!(out.headers.get("x-hook-response").unwrap(), "ok");
        assert_eq!(out.body, json!({"ok":false}));
    }
}
