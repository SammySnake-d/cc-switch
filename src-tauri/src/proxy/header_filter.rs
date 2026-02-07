/// Headers 黑名单 - 不透传到上游的 Headers
///
/// 精简版黑名单，只过滤必须覆盖或可能导致问题的 header
/// 参考成功透传的请求，保留更多原始 header
///
/// 注意：客户端 IP 类（x-forwarded-for, x-real-ip）默认透传（由 forwarder 单独处理）
pub(crate) const HEADER_BLACKLIST: &[&str] = &[
    // 认证类（会被覆盖）
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    // 连接类（由 HTTP 客户端管理）
    "host",
    "content-length",
    "transfer-encoding",
    // 编码类（会被覆盖为 identity）
    "accept-encoding",
    // 代理转发类（保留 x-forwarded-for 和 x-real-ip）
    "x-forwarded-host",
    "x-forwarded-port",
    "x-forwarded-proto",
    "forwarded",
    // CDN/云服务商特定头
    "cf-connecting-ip",
    "cf-ipcountry",
    "cf-ray",
    "cf-visitor",
    "true-client-ip",
    "fastly-client-ip",
    "x-azure-clientip",
    "x-azure-fdid",
    "x-azure-ref",
    "akamai-origin-hop",
    "x-akamai-config-log-detail",
    // 请求追踪类
    "x-request-id",
    "x-correlation-id",
    "x-trace-id",
    "x-amzn-trace-id",
    "x-b3-traceid",
    "x-b3-spanid",
    "x-b3-parentspanid",
    "x-b3-sampled",
    "traceparent",
    "tracestate",
    // anthropic 特定头单独处理，避免重复
    "anthropic-beta",
    "anthropic-version",
    // 客户端 IP 单独处理（默认透传）
    "x-forwarded-for",
    "x-real-ip",
];

pub(crate) fn is_header_blacklisted(name: &str) -> bool {
    HEADER_BLACKLIST
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
}
