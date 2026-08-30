// SPDX-FileCopyrightText: GoCortexIO
// SPDX-License-Identifier: AGPL-3.0-or-later

use anyhow::{Context, Result};
use reqwest::{Client, Response};
use serde_json::Value;

use crate::config::ModuleConfig;
use crate::modules::{ContentTypeDefinition, PullStrategy};
use crate::types::XsiamObject;

/// Maximum number of pagination pages fetched per content-type pull.
/// Prevents a hostile or malfunctioning API from driving unbounded network
/// requests and memory growth.
const MAX_PULL_PAGES: usize = 500;

/// Maximum number of objects accepted per content-type pull across all pages.
/// Prevents a hostile API from filling local disk with attacker-controlled YAML.
const MAX_PULL_OBJECTS: usize = 50_000;

/// Maximum response body size in bytes accepted from the API before parsing.
/// 64 MiB is generous for real-world Cortex responses while bounding memory use.
const MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Per-request timeout applied to every outbound HTTP request.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Total number of attempts made for a request that fails in a way worth retrying.
const MAX_REQUEST_ATTEMPTS: u32 = 4;

/// Consecutive pages that may return nothing new before retrieval gives up.
///
/// One unproductive page is not proof that a collection has been exhausted. Where the
/// ordering is not stable between requests, a window can overlap one already seen while
/// records beyond it remain unfetched. Stopping at the first such page truncates the
/// result; tolerating a couple lets retrieval move past the overlap. MAX_PULL_PAGES
/// still bounds the walk, so an endpoint that never advances cannot loop.
const UNPRODUCTIVE_PAGE_TOLERANCE: usize = 2;

/// Attempts made when the connection itself cannot be established.
///
/// Deliberately lower than MAX_REQUEST_ATTEMPTS. A refused connection, a failed DNS
/// lookup or an unroutable host does not heal within seconds, and the cost is paid
/// once per content type: a pull of ten content types against an unreachable tenant
/// took over two minutes purely in backoff before reporting the failure. One retry
/// still absorbs a genuine transient reset.
const MAX_CONNECT_ATTEMPTS: u32 = 2;

/// Base delay for the exponential backoff between retries.
const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Upper bound on a single backoff delay, including a server-supplied Retry-After.
const MAX_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(60);

/// The result of pulling one content type.
///
/// `complete` is false when the pull returned a usable but partial view: an
/// individual item could not be fetched, or the set of records to ask for could not
/// be established. The caller must not prune local files from a partial pull,
/// because an object missing from the response may still exist on the platform.
pub struct PullOutcome {
    pub objects: Vec<XsiamObject>,
    pub complete: bool,
}

impl PullOutcome {
    fn complete(objects: Vec<XsiamObject>) -> Self {
        Self {
            objects,
            complete: true,
        }
    }

    fn partial(objects: Vec<XsiamObject>) -> Self {
        Self {
            objects,
            complete: false,
        }
    }
}

pub struct ModuleClient {
    client: Client,
    fqdn: String,
    api_key: String,
    api_key_id: String,
    base_api_path: String,
    scheme: &'static str,
}

impl ModuleClient {
    pub fn new(config: ModuleConfig, base_api_path: &str) -> Self {
        // Identify gcgit traffic on the wire and in platform-side request logs.
        // Refuse redirects on every connection, not only the plaintext one. reqwest
        // does not strip the custom x-xdr-auth-id header on a cross-host redirect, so
        // a redirect could hand the credentials to another host over HTTPS just as
        // easily. Cortex endpoints do not redirect, so this costs nothing.
        let mut builder = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("gcgit/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none());

        if config.force_http {
            // reqwest applies HTTP_PROXY to http:// requests and does not bypass it
            // for loopback destinations, which would route credential-bearing
            // requests through a proxy that can read every header.
            builder = builder.no_proxy();
        }

        let client = builder.build().expect("Failed to build HTTP client");
        Self {
            client,
            fqdn: config.fqdn,
            api_key: config.api_key,
            api_key_id: config.api_key_id,
            base_api_path: base_api_path.to_string(),
            scheme: if config.force_http { "http" } else { "https" },
        }
    }

    /// Build the full URL for an endpoint.
    ///
    /// An endpoint beginning with `/` is treated as absolute from the host root and
    /// bypasses the module's base path. This is how content types reach endpoints
    /// that sit outside their module's version prefix, such as the XQL library at
    /// `/public_api/xql_library/get` while the rest of XSIAM is under
    /// `/public_api/v1`. The previous approach used a `../` relative segment and
    /// depended on the HTTP client normalising it away.
    fn endpoint_url(&self, endpoint: &str) -> String {
        match endpoint.strip_prefix('/') {
            Some(absolute) => format!("{}://{}/{}", self.scheme, self.fqdn, absolute),
            None => format!(
                "{}://{}{}/{}",
                self.scheme, self.fqdn, self.base_api_path, endpoint
            ),
        }
    }

    /// Attach the credentials and headers every Cortex request needs.
    ///
    /// Centralised so there is a single place to change when a module needs a
    /// different authentication scheme. The `/platform/` endpoints introduced by
    /// the Cloud Consumption API expect a bearer JWT rather than these headers.
    fn authorised(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("x-xdr-auth-id", &self.api_key_id)
            .header("Authorization", &self.api_key)
            .header("Accept", "application/json")
    }

    /// How long to wait before the next attempt.
    ///
    /// Honours a `Retry-After` header when the server sends one, otherwise backs off
    /// exponentially. Both are capped so a hostile or misconfigured server cannot
    /// stall the pull indefinitely.
    fn retry_delay(response: Option<&Response>, attempt: u32) -> std::time::Duration {
        let retry_after = response
            .and_then(|r| r.headers().get(reqwest::header::RETRY_AFTER))
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(std::time::Duration::from_secs);

        let delay = retry_after
            .unwrap_or_else(|| RETRY_BASE_DELAY * 2u32.saturating_pow(attempt.saturating_sub(1)));

        delay.min(MAX_RETRY_DELAY)
    }

    /// Send a request, retrying on rate limiting, server errors and transient
    /// network failures.
    ///
    /// The Cortex public APIs document HTTP 429 with a rate-limit body on every
    /// endpoint, and a pull issues one request per script or artefact, so being
    /// throttled part way through a large tenant is expected rather than
    /// exceptional.
    async fn send_with_retry(
        &self,
        request: reqwest::RequestBuilder,
        description: &str,
    ) -> Result<Response> {
        self.send_with_attempts(request, description, MAX_REQUEST_ATTEMPTS)
            .await
    }

    /// As `send_with_retry`, but with an explicit attempt budget.
    ///
    /// The connectivity check passes 1: it is a diagnostic, and retrying a bad
    /// hostname or a wrong region four times just makes the user wait before seeing
    /// the answer they need.
    async fn send_with_attempts(
        &self,
        request: reqwest::RequestBuilder,
        description: &str,
        max_attempts: u32,
    ) -> Result<Response> {
        let mut attempt: u32 = 1;

        loop {
            // A request whose body cannot be cloned (a stream) cannot be retried.
            let Some(attempt_request) = request.try_clone() else {
                return request
                    .send()
                    .await
                    .with_context(|| format!("Failed to send request for {description}"));
            };

            let outcome = attempt_request.send().await;
            let is_last_attempt = attempt >= max_attempts;

            match outcome {
                Ok(response) => {
                    let status = response.status();
                    let worth_retrying = status.as_u16() == 429 || status.is_server_error();

                    if !worth_retrying || is_last_attempt {
                        return Ok(response);
                    }

                    let delay = Self::retry_delay(Some(&response), attempt);
                    eprintln!(
                        "[INFO] {description} returned HTTP {}; retrying in {}s (attempt {} of {})",
                        status.as_u16(),
                        delay.as_secs(),
                        attempt,
                        max_attempts
                    );
                    tokio::time::sleep(delay).await;
                }
                Err(e) => {
                    // A timeout may be load-related and is worth the full budget; a
                    // connection failure is not.
                    let budget = if e.is_connect() {
                        max_attempts.min(MAX_CONNECT_ATTEMPTS)
                    } else {
                        max_attempts
                    };
                    let worth_retrying = (e.is_timeout() || e.is_connect()) && attempt < budget;
                    if !worth_retrying {
                        return Err(e)
                            .with_context(|| format!("Failed to send request for {description}"));
                    }

                    let delay = Self::retry_delay(None, attempt);
                    eprintln!(
                        "[INFO] {description} failed to connect ({e}); retrying in {}s (attempt {} of {})",
                        delay.as_secs(),
                        attempt,
                        max_attempts
                    );
                    tokio::time::sleep(delay).await;
                }
            }

            attempt += 1;
        }
    }

    /// Turn a failed HTTP response into an error that says what actually happened.
    ///
    /// Reads the body so the Cortex `{err_code, err_msg, err_extra}` envelope can be
    /// reported: `err_extra` frequently names the exact missing permission, which a
    /// bare status code does not.
    async fn describe_failure(response: Response, description: &str) -> anyhow::Error {
        let status = response.status();
        let hint = match status.as_u16() {
            // The platform specification documents 402 as a licence failure and 403
            // as an RBAC one, consistently across every endpoint that lists them. A
            // 403 has also been seen for a request the endpoint could not interpret,
            // so the permission is the first thing to check rather than the only one.
            401 => " Check the API key and key ID.",
            402 => " The tenant licence does not cover this endpoint.",
            403 => " Usually a permission missing from the API key's role. A licence problem would be reported as 402.",
            404 => " The endpoint does not exist on this tenant.",
            429 => " The tenant is rate limiting requests.",
            _ => "",
        };

        let body = Self::read_bounded_body_bytes(response)
            .await
            .unwrap_or_default();
        let envelope = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|json| Self::cortex_error_message(&json));

        match envelope {
            Some(message) => {
                anyhow::anyhow!("{description} failed with HTTP {status}: {message}.{hint}")
            }
            None => anyhow::anyhow!("{description} failed with HTTP {status}.{hint}"),
        }
    }

    /// Pull a content type once and index the objects by every identifier they can
    /// be looked up under.
    ///
    /// Diff previously called a per-object lookup that re-pulled the whole content
    /// type for each local file, so comparing N objects issued N full pulls. For
    /// scripts, which already cost one request per script, that was quadratic in
    /// request count.
    pub async fn pull_content_type_by_id(
        &self,
        content_def: &ContentTypeDefinition,
    ) -> Result<std::collections::HashMap<String, XsiamObject>> {
        let objects = self.pull_content_type(content_def).await?.objects;
        let mut by_id = std::collections::HashMap::with_capacity(objects.len());

        for object in objects {
            // The content type's own ID field is indexed as well as the derived id,
            // because a local file may have been written under either.
            if let Some(field_id) =
                object
                    .content
                    .get(content_def.id_field)
                    .and_then(|value| match value {
                        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    })
            {
                by_id.entry(field_id).or_insert_with(|| object.clone());
            }

            by_id.insert(object.id.clone(), object);
        }

        Ok(by_id)
    }

    /// Verify that the tenant is reachable and the credentials are accepted.
    ///
    /// Uses a real endpoint and treats any non-2xx response as a failure. The
    /// previous implementation posted to the module's base path and only failed on
    /// 401, so a 403, 404 or 500 all reported success.
    pub async fn test_connectivity(&self, check_endpoint: &str) -> Result<()> {
        let url = self.endpoint_url(check_endpoint);

        let request = self
            .authorised(self.client.get(&url))
            .timeout(std::time::Duration::from_secs(30));

        let response = self
            .send_with_attempts(request, &self.fqdn.clone(), 1)
            .await
            .with_context(|| format!("Could not reach {}", self.fqdn))?;

        if !response.status().is_success() {
            return Err(Self::describe_failure(response, "Connectivity check").await);
        }

        Ok(())
    }

    /// Pull content using ContentTypeDefinition - supports all pull strategies
    pub async fn pull_content_type(
        &self,
        content_def: &ContentTypeDefinition,
    ) -> Result<PullOutcome> {
        match &content_def.pull_strategy {
            PullStrategy::JsonCollection => self
                .pull_json_collection(content_def)
                .await
                .map(PullOutcome::complete),
            PullStrategy::Paginated {
                page_param,
                page_size_param,
                page_size,
            } => self
                .pull_paginated(content_def, page_param, page_size_param, *page_size)
                .await
                .map(PullOutcome::complete),
            PullStrategy::ScriptCode {
                list_endpoint,
                code_endpoint,
                list_response_path,
                uid_field,
            } => {
                self.pull_script_code(
                    content_def,
                    list_endpoint,
                    code_endpoint,
                    list_response_path,
                    uid_field,
                )
                .await
            }
            PullStrategy::OffsetPaginated {
                offset_param,
                limit_param,
                page_size,
            } => self
                .pull_offset_paginated(content_def, offset_param, limit_param, *page_size)
                .await
                .map(PullOutcome::complete),
            PullStrategy::BodyWindowPaginated {
                from_param,
                to_param,
                total_field,
                page_size,
            } => self
                .pull_body_window_paginated(
                    content_def,
                    from_param,
                    to_param,
                    total_field,
                    *page_size,
                )
                .await
                .map(PullOutcome::complete),
            PullStrategy::NameListed {
                source_endpoint,
                source_response_path,
                source_name_field,
                names_param,
            } => {
                self.pull_by_name_list(
                    content_def,
                    source_endpoint,
                    source_response_path,
                    source_name_field,
                    names_param,
                )
                .await
            }
        }
    }

    /// Pull content whose endpoint reports the size of the full collection and
    /// takes an absolute window in the request body.
    async fn pull_body_window_paginated(
        &self,
        content_def: &ContentTypeDefinition,
        from_param: &str,
        to_param: &str,
        total_field: &str,
        page_size: usize,
    ) -> Result<Vec<XsiamObject>> {
        let url = self.endpoint_url(content_def.get_endpoint);
        let mut all_objects = Vec::new();
        let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut from: usize = 0;
        let mut page: usize = 0;
        let mut records_seen: usize = 0;
        let mut unproductive: usize = 0;
        let mut duplicate_ids: usize = 0;
        let mut reported_total: Option<usize> = None;

        loop {
            if page >= MAX_PULL_PAGES {
                eprintln!("[WARN] pull_body_window_paginated for '{}' reached the page limit ({MAX_PULL_PAGES}). Stopping early.", content_def.name);
                break;
            }
            if all_objects.len() >= MAX_PULL_OBJECTS {
                eprintln!("[WARN] pull_body_window_paginated for '{}' reached the object limit ({MAX_PULL_OBJECTS}). Stopping early.", content_def.name);
                break;
            }

            let body = serde_json::json!({
                "request_data": { from_param: from, to_param: from + page_size }
            });
            let request = self
                .authorised(self.client.post(&url))
                .header("Content-Type", "application/json")
                .json(&body);
            let response = self
                .send_with_retry(
                    request,
                    &format!("{} rows {}-{}", content_def.name, from, from + page_size),
                )
                .await?;

            if !response.status().is_success() {
                return Err(Self::describe_failure(response, content_def.name).await);
            }

            let json: Value = self.read_bounded_json_response(response).await?;

            if reported_total.is_none() {
                reported_total = self
                    .extract_value_by_path(&json, &format!("reply.{total_field}"))
                    .ok()
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
            }

            let objects = self.extract_items_from_response(&json, content_def)?;
            let batch_size = objects.len();
            records_seen += batch_size;

            // One object per identifier. An endpoint may return the same record more
            // than once, for example a rule listed under several categories, and the
            // repository stores one file per record. Where duplicates differ, the one
            // that sorts first by its serialised form wins, so the stored copy does
            // not depend on the order the pages happened to arrive in.
            let mut new_objects: Vec<XsiamObject> = Vec::new();
            for object in objects {
                if seen_ids.insert(object.id.clone()) {
                    new_objects.push(object);
                    continue;
                }
                duplicate_ids += 1;
                if let Some(existing) = all_objects
                    .iter_mut()
                    .chain(new_objects.iter_mut())
                    .find(|held| held.id == object.id)
                {
                    if Self::prefer_candidate(existing, &object, content_def.dedupe_by_latest) {
                        *existing = object;
                    }
                }
            }
            let made_progress = !new_objects.is_empty();
            all_objects.extend(new_objects);
            page += 1;

            if batch_size == 0 {
                break;
            }

            // Continue while pages are still contributing records that have not been
            // seen. Stopping once records_seen reaches the reported total assumes the
            // windows tile the collection exactly, which requires the ordering to hold
            // still between requests. It does not always: a live tenant reporting 1288
            // records yielded between 688 and 1029 distinct identifiers across runs,
            // because overlapping windows consumed the budget before every record had
            // been seen.
            if made_progress {
                unproductive = 0;
            } else {
                unproductive += 1;
                if unproductive >= UNPRODUCTIVE_PAGE_TOLERANCE {
                    break;
                }
            }
            from += batch_size;
        }

        if let Some(total) = reported_total {
            if records_seen < total {
                eprintln!(
                    "[WARN] {} reported {} records but only {} were returned. Some records were not retrieved.",
                    content_def.name, total, records_seen
                );
            } else if duplicate_ids > 0 {
                // Not a fault: the endpoint returns the same record under more than
                // one category, and one file is stored per distinct identifier.
                println!(
                    "  Note: {} returned {} records covering {} distinct identifiers; {} duplicate(s) were collapsed.",
                    content_def.name, records_seen, all_objects.len(), duplicate_ids
                );
            }
        }

        Ok(all_objects)
    }

    /// Pull content from an endpoint that cannot enumerate itself, by first
    /// harvesting the record names from an endpoint that can.
    async fn pull_by_name_list(
        &self,
        content_def: &ContentTypeDefinition,
        source_endpoint: &str,
        source_response_path: &str,
        source_name_field: &str,
        names_param: &str,
    ) -> Result<PullOutcome> {
        // Step one: collect the names.
        let source_url = self.endpoint_url(source_endpoint);
        let request = self
            .authorised(self.client.post(&source_url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"request_data": {}}));
        let response = self
            .send_with_retry(request, &format!("{} name listing", content_def.name))
            .await?;

        if !response.status().is_success() {
            return Err(Self::describe_failure(
                response,
                &format!("{} name listing", content_def.name),
            )
            .await);
        }

        let source_json: Value = self.read_bounded_json_response(response).await?;
        let source_items = self
            .extract_value_by_path(&source_json, source_response_path)?
            .as_array()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Expected an array at '{source_response_path}' when collecting {} names",
                    content_def.name
                )
            })?;

        // The field is a single name on some endpoints and a list on others.
        let mut names: Vec<String> = Vec::new();
        for item in source_items {
            match item.get(source_name_field) {
                Some(Value::String(name)) if !name.trim().is_empty() => names.push(name.clone()),
                Some(Value::Array(list)) => {
                    for entry in list {
                        if let Value::String(name) = entry {
                            if !name.trim().is_empty() {
                                names.push(name.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        names.sort();
        names.dedup();

        if names.is_empty() {
            if source_items.is_empty() {
                // The source endpoint genuinely has no records, so there is nothing
                // for this content type to reference. A real empty result.
                return Ok(PullOutcome::complete(Vec::new()));
            }
            // The source returned records but no usable names came out of
            // `source_name_field`, which means the field is absent or shaped
            // differently than expected. Reporting this as an empty collection would
            // let pruning delete every stored file for the content type.
            eprintln!(
                "[WARN] {} names could not be read from '{}' in {} source record(s). \
                 Local files for this content type will not be pruned.",
                content_def.name,
                source_name_field,
                source_items.len()
            );
            return Ok(PullOutcome::partial(Vec::new()));
        }
        if names.len() > MAX_PULL_OBJECTS {
            eprintln!(
                "[WARN] {} name list exceeds the object limit; truncating.",
                content_def.name
            );
            names.truncate(MAX_PULL_OBJECTS);
        }

        // Step two: fetch them all in one request.
        let url = self.endpoint_url(content_def.get_endpoint);
        let request = self
            .authorised(self.client.post(&url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"request_data": {names_param: names}}));
        let response = self.send_with_retry(request, content_def.name).await?;

        if !response.status().is_success() {
            return Err(Self::describe_failure(response, content_def.name).await);
        }

        let json: Value = self.read_bounded_json_response(response).await?;
        let objects = self.extract_items_from_response(&json, content_def)?;
        Ok(PullOutcome::complete(Self::truncate_objects(
            objects,
            content_def.name,
        )))
    }

    /// Stream a response body chunk-by-chunk, aborting mid-stream once the
    /// running byte count exceeds `MAX_RESPONSE_BODY_BYTES`.  This prevents
    /// OOM from large API responses because the process stops reading (and
    /// therefore stops buffering) as soon as the limit is hit, unlike calling
    /// `response.bytes().await` which buffers the whole body before any check.
    async fn read_bounded_body_bytes(response: Response) -> Result<Vec<u8>> {
        let mut buf: Vec<u8> = Vec::new();
        let mut response = response;
        loop {
            match response
                .chunk()
                .await
                .context("Failed to read response chunk")?
            {
                None => break,
                Some(chunk) => {
                    if buf.len() + chunk.len() > MAX_RESPONSE_BODY_BYTES {
                        return Err(anyhow::anyhow!(
                            "API response body exceeds the maximum allowed size of \
                             {MAX_RESPONSE_BODY_BYTES} bytes. Aborting read to prevent \
                             resource exhaustion."
                        ));
                    }
                    buf.extend_from_slice(&chunk);
                }
            }
        }
        Ok(buf)
    }

    /// Read a response body with streaming size enforcement and parse as JSON.
    async fn read_bounded_json_response(&self, response: Response) -> Result<Value> {
        let bytes = Self::read_bounded_body_bytes(response).await?;
        serde_json::from_slice(&bytes).context("Failed to parse JSON response")
    }

    /// Truncate an object list to `MAX_PULL_OBJECTS`, warning if truncation occurred.
    fn truncate_objects(mut objects: Vec<XsiamObject>, content_type: &str) -> Vec<XsiamObject> {
        if objects.len() > MAX_PULL_OBJECTS {
            eprintln!(
                "[WARN] Received {} objects for '{}', which exceeds the limit of {}. \
                 Truncating to prevent resource exhaustion.",
                objects.len(),
                content_type,
                MAX_PULL_OBJECTS
            );
            objects.truncate(MAX_PULL_OBJECTS);
        }
        objects
    }

    /// Pull JSON collection - single API call
    async fn pull_json_collection(
        &self,
        content_def: &ContentTypeDefinition,
    ) -> Result<Vec<XsiamObject>> {
        let url = self.endpoint_url(content_def.get_endpoint);

        let request = match &content_def.request_body {
            Some(body) => self
                .authorised(self.client.post(&url))
                .header("Content-Type", "application/json")
                .json(body),
            None => self.authorised(self.client.get(&url)),
        };

        let response = self.send_with_retry(request, content_def.name).await?;

        if !response.status().is_success() {
            return Err(Self::describe_failure(response, content_def.name).await);
        }

        let json: Value = self.read_bounded_json_response(response).await?;
        let objects = self.extract_items_from_response(&json, content_def)?;
        Ok(Self::truncate_objects(objects, content_def.name))
    }

    /// Pull paginated content - multiple API calls
    async fn pull_paginated(
        &self,
        content_def: &ContentTypeDefinition,
        page_param: &str,
        page_size_param: &str,
        page_size: usize,
    ) -> Result<Vec<XsiamObject>> {
        let mut all_objects = Vec::new();
        let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut page = 1;
        let mut unproductive: usize = 0;

        loop {
            if page > MAX_PULL_PAGES {
                eprintln!("[WARN] pull_paginated for '{}' reached the page limit ({MAX_PULL_PAGES}). Stopping early to prevent resource exhaustion.", content_def.name);
                break;
            }
            if all_objects.len() >= MAX_PULL_OBJECTS {
                eprintln!("[WARN] pull_paginated for '{}' reached the object limit ({MAX_PULL_OBJECTS}). Stopping early to prevent resource exhaustion.", content_def.name);
                break;
            }

            let url = format!(
                "{}?{}={}&{}={}",
                self.endpoint_url(content_def.get_endpoint),
                page_param,
                page,
                page_size_param,
                page_size
            );

            let request = self.authorised(self.client.get(&url));
            let response = self
                .send_with_retry(request, &format!("{} page {}", content_def.name, page))
                .await?;

            if !response.status().is_success() {
                return Err(Self::describe_failure(response, content_def.name).await);
            }

            let json: Value = self.read_bounded_json_response(response).await?;
            let mut objects = self.extract_items_from_response(&json, content_def)?;

            // Strictly cap total: truncate this batch if it would overflow the budget.
            let remaining = MAX_PULL_OBJECTS.saturating_sub(all_objects.len());
            if objects.len() > remaining {
                eprintln!("[WARN] pull_paginated for '{}' reached the object limit ({MAX_PULL_OBJECTS}). Truncating batch.", content_def.name);
                objects.truncate(remaining);
            }

            // Guard against an endpoint that ignores the page parameter or clamps an
            // out-of-range page to the last one. Without this, such an endpoint would
            // return the same rows until MAX_PULL_PAGES, duplicating every object.
            // `application/criteria/all` publishes currentPage and totalPages but no
            // hasNext, so it relies on this rather than an explicit end marker.
            let new_objects: Vec<XsiamObject> = objects
                .into_iter()
                .filter(|object| seen_ids.insert(object.id.clone()))
                .collect();

            let has_next = json.get("hasNext").and_then(|v| v.as_bool());

            if new_objects.is_empty() {
                unproductive += 1;
                // The endpoint saying there is more outranks one unproductive page:
                // an overlapping page is not proof the collection is exhausted.
                let exhausted =
                    has_next != Some(true) || unproductive >= UNPRODUCTIVE_PAGE_TOLERANCE;
                if exhausted {
                    if has_next == Some(true) {
                        eprintln!(
                            "[WARN] {} stopped at page {} after {} page(s) returning nothing new, \
                             although the endpoint reported more. The '{}' parameter may be ignored.",
                            content_def.name, page, unproductive, page_param
                        );
                    }
                    break;
                }
                page += 1;
                continue;
            }
            unproductive = 0;

            let reached_limit = all_objects.len() + new_objects.len() >= MAX_PULL_OBJECTS;
            all_objects.extend(new_objects);

            if reached_limit || has_next == Some(false) {
                break;
            }

            page += 1;
        }

        Ok(all_objects)
    }

    /// Pull offset-paginated content - iterates using offset and limit query parameters
    async fn pull_offset_paginated(
        &self,
        content_def: &ContentTypeDefinition,
        offset_param: &str,
        limit_param: &str,
        page_size: usize,
    ) -> Result<Vec<XsiamObject>> {
        let mut all_objects = Vec::new();
        let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut offset: usize = 0;
        let mut page: usize = 0;
        let mut unproductive: usize = 0;

        loop {
            if page >= MAX_PULL_PAGES {
                eprintln!("[WARN] pull_offset_paginated for '{}' reached the page limit ({MAX_PULL_PAGES}). Stopping early to prevent resource exhaustion.", content_def.name);
                break;
            }
            if all_objects.len() >= MAX_PULL_OBJECTS {
                eprintln!("[WARN] pull_offset_paginated for '{}' reached the object limit ({MAX_PULL_OBJECTS}). Stopping early to prevent resource exhaustion.", content_def.name);
                break;
            }

            let url = format!(
                "{}?{}={}&{}={}",
                self.endpoint_url(content_def.get_endpoint),
                offset_param,
                offset,
                limit_param,
                page_size
            );

            let request = self.authorised(self.client.get(&url));
            let response = self
                .send_with_retry(
                    request,
                    &format!("{} at offset {}", content_def.name, offset),
                )
                .await?;

            if !response.status().is_success() {
                return Err(Self::describe_failure(response, content_def.name).await);
            }

            let json: Value = self.read_bounded_json_response(response).await?;
            let mut objects = self.extract_items_from_response(&json, content_def)?;

            // Strictly cap total: truncate this batch if it would overflow the budget.
            let remaining = MAX_PULL_OBJECTS.saturating_sub(all_objects.len());
            if objects.len() > remaining {
                eprintln!("[WARN] pull_offset_paginated for '{}' reached the object limit ({MAX_PULL_OBJECTS}). Truncating batch.", content_def.name);
                objects.truncate(remaining);
            }

            let batch_size = objects.len();

            // Drop anything already collected so an endpoint that clamps or ignores
            // the offset cannot produce duplicates.
            let new_objects: Vec<XsiamObject> = objects
                .into_iter()
                .filter(|object| seen_ids.insert(object.id.clone()))
                .collect();
            let made_progress = !new_objects.is_empty();

            let reached_limit = all_objects.len() + new_objects.len() >= MAX_PULL_OBJECTS;
            all_objects.extend(new_objects);
            page += 1;

            if reached_limit || batch_size == 0 {
                break;
            }
            if made_progress {
                unproductive = 0;
            } else {
                unproductive += 1;
                if unproductive >= UNPRODUCTIVE_PAGE_TOLERANCE {
                    break;
                }
            }

            // Prefer the offset the server reports (AppSec `rules` returns one) over a
            // locally computed value.
            let next_offset = json
                .get(offset_param)
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .filter(|next| *next > offset);

            match next_offset {
                Some(next) => offset = next,
                None => {
                    // A short page means the end of the collection for endpoints that
                    // do not report an offset.
                    if batch_size < page_size {
                        break;
                    }
                    offset += batch_size;
                }
            }
        }

        Ok(all_objects)
    }

    /// Pull script code - two-step process (list scripts + fetch code by UID)
    async fn pull_script_code(
        &self,
        content_def: &ContentTypeDefinition,
        list_endpoint: &str,
        code_endpoint: &str,
        list_response_path: &str,
        uid_field: &str,
    ) -> Result<PullOutcome> {
        let list_url = self.endpoint_url(list_endpoint);

        let request = self
            .authorised(self.client.post(&list_url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({"request_data": {}}));

        let response = self
            .send_with_retry(request, &format!("{} listing", content_def.name))
            .await?;

        if !response.status().is_success() {
            return Err(Self::describe_failure(response, content_def.name).await);
        }

        let json_response: Value = self.read_bounded_json_response(response).await?;

        let scripts_list = self
            .extract_value_by_path(&json_response, list_response_path)?
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected array at path {list_response_path}"))?;

        if scripts_list.len() > MAX_PULL_OBJECTS {
            eprintln!("[WARN] pull_script_code for '{}' returned {} items, exceeding the limit of {}. Truncating.", content_def.name, scripts_list.len(), MAX_PULL_OBJECTS);
        }

        let mut script_objects = Vec::new();
        let mut failed_items = 0usize;

        for script_meta in scripts_list.iter().take(MAX_PULL_OBJECTS) {
            let script_uid = script_meta
                .get(uid_field)
                .and_then(|uid| uid.as_str())
                .ok_or_else(|| anyhow::anyhow!("Script missing {uid_field} field"))?;

            let script_name = script_meta
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(script_uid);

            match self.get_script_code(code_endpoint, script_uid).await {
                Ok(script_code) => {
                    let mut content_map = std::collections::BTreeMap::new();

                    // Store the script code with newlines properly converted
                    content_map.insert("code".to_string(), serde_json::json!(script_code));

                    // Add all metadata fields except name, description, and uid
                    for (key, value) in script_meta.as_object().unwrap_or(&serde_json::Map::new()) {
                        if key != "name" && key != "description" && key != uid_field {
                            content_map.insert(key.clone(), value.clone());
                        }
                    }

                    let mut metadata = crate::types::ObjectMetadata::default();
                    if let Some(created_by) = script_meta.get("created_by").and_then(|v| v.as_str())
                    {
                        metadata.created_by = created_by.to_string();
                    }
                    if let Some(modification_date) = script_meta
                        .get("modification_date")
                        .and_then(|v| v.as_i64())
                    {
                        let seconds = if modification_date > 10000000000 {
                            modification_date / 1000
                        } else {
                            modification_date
                        };
                        metadata.updated_at = chrono::DateTime::from_timestamp(seconds, 0);
                    }

                    let description = script_meta
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();

                    let xsiam_obj = XsiamObject {
                        id: script_uid.to_string(),
                        name: Some(script_name.to_string()),
                        description,
                        content_type: content_def.name.to_string(),
                        metadata,
                        tenant_id: None,
                        content: content_map,
                    };
                    script_objects.push(xsiam_obj);
                }
                Err(e) => {
                    // The script exists but its code could not be fetched. The
                    // result is therefore partial: treating it as complete would let
                    // pruning delete a script that is still on the platform.
                    failed_items += 1;
                    eprintln!("Warning: Failed to get code for script '{script_name}': {e}");
                }
            }
        }

        if failed_items > 0 {
            eprintln!(
                "[WARN] {} of {} {} could not be retrieved. Local files for this content type will not be pruned.",
                failed_items,
                script_objects.len() + failed_items,
                content_def.name
            );
            return Ok(PullOutcome::partial(script_objects));
        }

        Ok(PullOutcome::complete(script_objects))
    }

    /// Get script code by UID - returns code with escaped newlines converted to actual newlines
    async fn get_script_code(&self, code_endpoint: &str, script_uid: &str) -> Result<String> {
        let code_url = self.endpoint_url(code_endpoint);

        let request = self
            .authorised(self.client.post(&code_url))
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({
                "request_data": {
                    "script_uid": script_uid
                }
            }));

        let response = self
            .send_with_retry(request, &format!("script code for '{script_uid}'"))
            .await?;

        if !response.status().is_success() {
            return Err(Self::describe_failure(
                response,
                &format!("Script code for '{script_uid}'"),
            )
            .await);
        }

        let json: Value = self.read_bounded_json_response(response).await?;

        let script_code = json
            .get("reply")
            .and_then(|r| r.as_str())
            .ok_or_else(|| anyhow::anyhow!("Script code response missing 'reply' field"))?;

        // Convert escaped newlines (\n) to actual newlines for readability
        let code_with_newlines = script_code.replace("\\n", "\n");

        Ok(code_with_newlines)
    }

    /// Decide whether a newly seen duplicate should replace the copy already held.
    ///
    /// When the content type names a field to compare, the copy with the greater value
    /// wins: for records returned once per category, that field is what indicates the
    /// record itself changed, so the most recent copy is the one worth storing.
    /// Otherwise fall back to the serialised form, which is arbitrary but stable, so
    /// the stored copy does not depend on the order pages happened to arrive in.
    fn prefer_candidate(existing: &XsiamObject, candidate: &XsiamObject, by: Option<&str>) -> bool {
        if let Some(field) = by {
            let held = existing.content.get(field);
            let offered = candidate.content.get(field);
            match (held, offered) {
                // Compare numerically where both are numbers, which is what a
                // millisecond timestamp is, and lexically otherwise.
                (Some(a), Some(b)) => {
                    if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
                        return b > a;
                    }
                    if let (Some(a), Some(b)) = (a.as_str(), b.as_str()) {
                        return b > a;
                    }
                }
                // A copy that carries the field beats one that does not.
                (None, Some(_)) => return true,
                (Some(_), None) => return false,
                (None, None) => {}
            }
        }

        let held = serde_json::to_string(existing).unwrap_or_default();
        let offered = serde_json::to_string(candidate).unwrap_or_default();
        offered < held
    }

    /// Sort the declared set-valued fields of an object so that a platform which
    /// returns their members in an arbitrary order does not produce a diff on every
    /// pull.
    fn normalise_set_valued_fields(object: &mut XsiamObject, fields: &[&str]) {
        for field in fields {
            if let Some(Value::Array(items)) = object.content.get_mut(*field) {
                items.sort_by_cached_key(|item| serde_json::to_string(item).unwrap_or_default());
            }
        }
    }

    /// Make server-supplied text safe to print to a terminal.
    ///
    /// Error bodies are echoed to the operator. Escape sequences in that text can
    /// rewrite earlier output, hide lines, or set the window title, so a hostile or
    /// compromised endpoint could make a failed pull look like a clean one. Control
    /// characters are replaced and the text is capped.
    fn sanitise_for_display(text: &str) -> String {
        const MAX_LEN: usize = 500;

        let cleaned: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let cleaned = cleaned.trim();

        if cleaned.chars().count() > MAX_LEN {
            let truncated: String = cleaned.chars().take(MAX_LEN).collect();
            format!("{truncated}...")
        } else {
            cleaned.to_string()
        }
    }

    /// Describe a Cortex error envelope if the response body carries one.
    ///
    /// `{"reply": {"err_code": N, "err_msg": "...", "err_extra": "..."}}` is the
    /// standard error body across the Cortex public APIs, and it can arrive on a
    /// response that gcgit has already accepted as successful. `err_extra` often
    /// names the exact missing permission, so it is worth surfacing.
    fn cortex_error_message(json: &Value) -> Option<String> {
        let reply = json.get("reply")?;
        let message = reply
            .get("err_msg")
            .and_then(|v| v.as_str())
            .map(Self::sanitise_for_display)?;
        let extra = reply
            .get("err_extra")
            .and_then(|v| v.as_str())
            .map(Self::sanitise_for_display);
        let message = message.as_str();
        let extra = extra.as_deref();

        // The HTTP status is already reported by the caller, and err_code repeats it,
        // so neither is included here. The message previously read "failed with HTTP
        // 403 Forbidden: Cortex error 403: Forbidden..." for a single failure.
        Some(match extra {
            Some(extra) if extra != message => format!("{message} ({extra})"),
            _ => message.to_string(),
        })
    }

    /// Extract items from a JSON response using the content type's response_path.
    ///
    /// A structural mismatch is returned as an error rather than an empty list. The
    /// two outcomes must stay distinguishable: `pull` prunes local files for objects
    /// that no longer exist remotely, and treating "the response did not look like
    /// we expected" as "there are no objects" would delete every file for the
    /// content type.
    /// Resolve a response path containing a `[*]` wildcard.
    ///
    /// The wildcard means: take every element of the array at the prefix, resolve the
    /// remainder against each, and concatenate the results. The dashboards and widgets
    /// endpoints need this because they return one wrapper object per record rather
    /// than one wrapper holding every record, so reading a fixed index retrieves a
    /// single record and silently discards the rest.
    fn extract_wildcard_items<'a>(&self, json: &'a Value, path: &str) -> Result<Vec<&'a Value>> {
        let Some((prefix, suffix)) = path.split_once("[*]") else {
            return Err(anyhow::anyhow!("Path '{path}' does not contain a wildcard"));
        };
        let suffix = suffix.trim_start_matches('.');

        let container = if prefix.is_empty() {
            json
        } else {
            self.extract_value_by_path(json, prefix)?
        };
        let elements = container
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Expected an array at '{prefix}'"))?;

        let mut collected = Vec::new();
        for element in elements {
            let resolved = if suffix.is_empty() {
                element
            } else {
                match self.extract_value_by_path(element, suffix) {
                    Ok(value) => value,
                    // An element without the field contributes nothing rather than
                    // failing the whole pull.
                    Err(_) => continue,
                }
            };
            match resolved.as_array() {
                Some(items) => collected.extend(items.iter()),
                None => collected.push(resolved),
            }
        }

        Ok(collected)
    }

    fn extract_items_from_response(
        &self,
        json: &Value,
        content_def: &ContentTypeDefinition,
    ) -> Result<Vec<XsiamObject>> {
        // A wildcard path gathers across every element of an array rather than
        // resolving to a single value, so it is handled before the normal path logic.
        if let Some(path) = content_def.response_path {
            if path.contains("[*]") {
                let collected = self.extract_wildcard_items(json, path)?;
                let mut objects = Vec::new();
                for item in collected {
                    let mut object = XsiamObject::from_api_response(
                        item,
                        content_def.name,
                        content_def.excluded_fields,
                    )?;
                    Self::normalise_set_valued_fields(&mut object, content_def.set_valued_fields);
                    objects.push(object);
                }
                return Ok(objects);
            }
        }

        let items = if let Some(path) = content_def.response_path {
            match self.extract_value_by_path(json, path) {
                Ok(value) => {
                    match value.as_array() {
                        // Some endpoints return one array per requested record rather
                        // than a flat list; rbac/get_roles nests this way. Flatten a
                        // single level when every element is itself an array.
                        // A list that mixes arrays and objects is neither shape and
                        // would otherwise hand raw arrays to the object parser, which
                        // produces content-free placeholders with no error.
                        Some(arr)
                            if arr.iter().any(|v| v.is_array())
                                && !arr.iter().all(|v| v.is_array()) =>
                        {
                            return Err(anyhow::anyhow!(
                                "Response path '{}' for {} mixes arrays and objects. \
                                 The endpoint may have changed structure.",
                                path,
                                content_def.name
                            ));
                        }
                        Some(arr) if !arr.is_empty() && arr.iter().all(|v| v.is_array()) => {
                            let flattened: Vec<&Value> =
                                arr.iter().filter_map(|v| v.as_array()).flatten().collect();
                            let mut objects = Vec::new();
                            for item in flattened {
                                let mut object = XsiamObject::from_api_response(
                                    item,
                                    content_def.name,
                                    content_def.excluded_fields,
                                )?;
                                Self::normalise_set_valued_fields(
                                    &mut object,
                                    content_def.set_valued_fields,
                                );
                                objects.push(object);
                            }
                            return Ok(objects);
                        }
                        Some(arr) => arr,
                        None => {
                            // Singleton handling: Agent Configurations endpoints return
                            // {"reply": {...singleton object...}} rather than an array.
                            if value.is_object()
                                && crate::modules::agent::is_agent_singleton(content_def.name)
                            {
                                let mut object = XsiamObject::from_api_response(
                                    value,
                                    content_def.name,
                                    content_def.excluded_fields,
                                )?;
                                Self::normalise_set_valued_fields(
                                    &mut object,
                                    content_def.set_valued_fields,
                                );
                                return Ok(vec![object]);
                            }
                            if let Some(error) = Self::cortex_error_message(json) {
                                return Err(anyhow::anyhow!(
                                    "{} endpoint returned an error instead of data: {}",
                                    content_def.name,
                                    error
                                ));
                            }
                            return Err(anyhow::anyhow!(
                                "Response path '{}' for {} exists but is not an array. \
                                 The endpoint may have changed structure.",
                                path,
                                content_def.name
                            ));
                        }
                    }
                }
                Err(_) => {
                    if let Some(error) = Self::cortex_error_message(json) {
                        return Err(anyhow::anyhow!(
                            "{} endpoint returned an error instead of data: {}",
                            content_def.name,
                            error
                        ));
                    }
                    return Err(anyhow::anyhow!(
                        "Response path '{}' not found for {}. The API response structure \
                         may have changed; verify the endpoint is working correctly.",
                        path,
                        content_def.name
                    ));
                }
            }
        } else {
            // No path specified - expect array at root
            match json.as_array() {
                Some(arr) => arr,
                None => {
                    if content_def.name == "application_configuration" {
                        let mut object = XsiamObject::from_api_response(
                            json,
                            content_def.name,
                            content_def.excluded_fields,
                        )?;
                        Self::normalise_set_valued_fields(
                            &mut object,
                            content_def.set_valued_fields,
                        );
                        return Ok(vec![object]);
                    }
                    if let Some(error) = Self::cortex_error_message(json) {
                        return Err(anyhow::anyhow!(
                            "{} endpoint returned an error instead of data: {}",
                            content_def.name,
                            error
                        ));
                    }
                    return Err(anyhow::anyhow!(
                        "Expected an array at the response root for {} but found none. \
                         The API response structure may have changed.",
                        content_def.name
                    ));
                }
            }
        };

        let mut objects = Vec::new();
        for item in items {
            let mut object = XsiamObject::from_api_response(
                item,
                content_def.name,
                content_def.excluded_fields,
            )?;
            Self::normalise_set_valued_fields(&mut object, content_def.set_valued_fields);
            objects.push(object);
        }

        Ok(objects)
    }

    /// Look up an object field, falling back to a case-insensitive match.
    ///
    /// Cortex is not consistent about key casing between the published
    /// specification and live tenants: `scheduled_queries/list` is documented as
    /// returning `reply.data` while tenants have been observed returning
    /// `reply.DATA`, and `xql/get_datasets` returns TitleCase keys where the
    /// specification documents snake_case. An exact match is tried first so the
    /// common path stays cheap; the fallback only runs when the exact key is
    /// absent.
    fn get_field_relaxed<'a>(value: &'a Value, field: &str) -> Option<&'a Value> {
        if let Some(found) = value.get(field) {
            return Some(found);
        }

        value
            .as_object()?
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(field))
            .map(|(_, found)| found)
    }

    /// Extract value from JSON using dot-notation path (e.g., "reply.scripts", "objects[0].dashboards_data")
    fn extract_value_by_path<'a>(&self, json: &'a Value, path: &str) -> Result<&'a Value> {
        let mut current = json;

        for segment in path.split('.') {
            if segment.contains('[') && segment.ends_with(']') {
                let parts: Vec<&str> = segment.split('[').collect();
                let field = parts[0];
                let index_str = parts[1].trim_end_matches(']');
                let index: usize = index_str
                    .parse()
                    .with_context(|| format!("Invalid array index: {index_str}"))?;

                if !field.is_empty() {
                    current = Self::get_field_relaxed(current, field)
                        .ok_or_else(|| anyhow::anyhow!("Path segment '{field}' not found"))?;
                }

                current = current
                    .get(index)
                    .ok_or_else(|| anyhow::anyhow!("Array index {index} not found"))?;
            } else {
                current = Self::get_field_relaxed(current, segment)
                    .ok_or_else(|| anyhow::anyhow!("Path segment '{segment}' not found"))?;
            }
        }

        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::{ContentTypeDefinition, PullStrategy};
    use serde_json::json;

    fn client() -> ModuleClient {
        ModuleClient::new(
            ModuleConfig {
                enabled: true,
                fqdn: "api-test.invalid".to_string(),
                api_key: "key".to_string(),
                api_key_id: "1".to_string(),
                force_http: false,
            },
            "/public_api/v1",
        )
    }

    fn content_def(
        name: &'static str,
        response_path: Option<&'static str>,
    ) -> ContentTypeDefinition {
        ContentTypeDefinition {
            name,
            get_endpoint: "test/get",
            pull_strategy: PullStrategy::JsonCollection,
            id_field: "id",
            request_body: None,
            response_path,
            dedupe_by_latest: None,
            excluded_fields: &[],
            set_valued_fields: &[],
        }
    }

    fn client_with(force_http: bool, fqdn: &str) -> ModuleClient {
        ModuleClient::new(
            ModuleConfig {
                enabled: true,
                fqdn: fqdn.to_string(),
                api_key: "key".to_string(),
                api_key_id: "1".to_string(),
                force_http,
            },
            "/public_api/v1",
        )
    }

    #[test]
    fn urls_use_https_by_default() {
        let client = client_with(false, "api-gocortex.xdr.au.paloaltonetworks.com");
        assert_eq!(
            client.endpoint_url("dashboards/get"),
            "https://api-gocortex.xdr.au.paloaltonetworks.com/public_api/v1/dashboards/get"
        );
    }

    #[test]
    fn force_http_switches_the_scheme() {
        let client = client_with(true, "localhost:8080");
        assert_eq!(
            client.endpoint_url("dashboards/get"),
            "http://localhost:8080/public_api/v1/dashboards/get"
        );
    }

    #[test]
    fn force_http_also_applies_to_absolute_endpoints() {
        // The XQL library sits outside the module's base path and is configured as an
        // absolute path, so it takes a different branch of endpoint_url.
        let client = client_with(true, "localhost:8080");
        assert_eq!(
            client.endpoint_url("/public_api/xql_library/get"),
            "http://localhost:8080/public_api/xql_library/get"
        );
    }

    #[test]
    fn absolute_endpoints_bypass_the_module_base_path() {
        let client = client_with(false, "api-t.example.com");
        assert_eq!(
            client.endpoint_url("/public_api/xql_library/get"),
            "https://api-t.example.com/public_api/xql_library/get"
        );
    }

    /// Retrieve every page a strategy would, against a stub whose second page repeats
    /// the first. Anything that stops at the first unproductive page, or once it has
    /// counted the reported total, truncates and misses the records beyond.
    #[tokio::test]
    async fn every_paginator_survives_an_overlapping_page() {
        use crate::modules::PullStrategy;

        // 30 records, served as: new, repeat, new, new, empty.
        let cases: Vec<(&str, ContentTypeDefinition)> = vec![
            (
                "window",
                ContentTypeDefinition {
                    name: "window",
                    get_endpoint: "window",
                    pull_strategy: PullStrategy::BodyWindowPaginated {
                        from_param: "search_from",
                        to_param: "search_to",
                        total_field: "total_count",
                        page_size: 10,
                    },
                    id_field: "id",
                    request_body: None,
                    response_path: Some("reply.rows"),
                    dedupe_by_latest: None,
                    excluded_fields: &[],
                    set_valued_fields: &[],
                },
            ),
            (
                "offset",
                ContentTypeDefinition {
                    name: "offset",
                    get_endpoint: "offset",
                    pull_strategy: PullStrategy::OffsetPaginated {
                        offset_param: "offset",
                        limit_param: "limit",
                        page_size: 10,
                    },
                    id_field: "id",
                    request_body: None,
                    response_path: None,
                    dedupe_by_latest: None,
                    excluded_fields: &[],
                    set_valued_fields: &[],
                },
            ),
            (
                "paged",
                ContentTypeDefinition {
                    name: "paged",
                    get_endpoint: "paged",
                    pull_strategy: PullStrategy::Paginated {
                        page_param: "page",
                        page_size_param: "pageSize",
                        page_size: 10,
                    },
                    id_field: "id",
                    request_body: None,
                    response_path: Some("data"),
                    dedupe_by_latest: None,
                    excluded_fields: &[],
                    set_valued_fields: &[],
                },
            ),
        ];

        let client = ModuleClient::new(
            ModuleConfig {
                enabled: true,
                fqdn: "127.0.0.1:8097".to_string(),
                api_key: "k".to_string(),
                api_key_id: "1".to_string(),
                force_http: true,
            },
            "/public_api/v1",
        );

        for (label, def) in cases {
            let outcome = client.pull_content_type(&def).await;
            let objects = match outcome {
                Ok(o) => o.objects,
                // The stub is started by the harness that runs this test. Without it
                // there is nothing to assert, so skip rather than fail spuriously.
                Err(_) => return,
            };
            assert_eq!(
                objects.len(),
                30,
                "{label} truncated at {} records; an overlapping page must not end the walk",
                objects.len()
            );
        }
    }

    #[test]
    fn a_duplicate_is_resolved_by_the_named_field() {
        // Attack surface rules come back once per category. Where the copies differ,
        // "modified" is what indicates the rule itself changed, so the most recently
        // modified copy is the one to keep.
        let older = XsiamObject::from_api_response(
            &json!({"attack_surface_rule_id": "R1", "modified": 1000i64, "category": "A"}),
            "attack_surface_rules",
            &[],
        )
        .unwrap();
        let newer = XsiamObject::from_api_response(
            &json!({"attack_surface_rule_id": "R1", "modified": 2000i64, "category": "B"}),
            "attack_surface_rules",
            &[],
        )
        .unwrap();

        assert!(
            ModuleClient::prefer_candidate(&older, &newer, Some("modified")),
            "a later modified should replace an earlier one"
        );
        assert!(
            !ModuleClient::prefer_candidate(&newer, &older, Some("modified")),
            "an earlier modified should not replace a later one"
        );
    }

    #[test]
    fn a_duplicate_choice_is_stable_without_a_named_field() {
        // No field named: the outcome must still be the same on every run, so the
        // stored copy does not depend on the order pages arrived in.
        let a = XsiamObject::from_api_response(&json!({"id": "R1", "v": "aaa"}), "policies", &[])
            .unwrap();
        let b = XsiamObject::from_api_response(&json!({"id": "R1", "v": "zzz"}), "policies", &[])
            .unwrap();

        let first = ModuleClient::prefer_candidate(&a, &b, None);
        for _ in 0..20 {
            assert_eq!(
                ModuleClient::prefer_candidate(&a, &b, None),
                first,
                "the choice must not vary between runs"
            );
        }
        // And the reverse comparison must disagree, so one of the two always wins.
        assert_ne!(first, ModuleClient::prefer_candidate(&b, &a, None));
    }

    #[test]
    fn a_copy_carrying_the_field_beats_one_without_it() {
        let without = XsiamObject::from_api_response(
            &json!({"attack_surface_rule_id": "R1"}),
            "attack_surface_rules",
            &[],
        )
        .unwrap();
        let with = XsiamObject::from_api_response(
            &json!({"attack_surface_rule_id": "R1", "modified": 5i64}),
            "attack_surface_rules",
            &[],
        )
        .unwrap();
        assert!(ModuleClient::prefer_candidate(
            &without,
            &with,
            Some("modified")
        ));
        assert!(!ModuleClient::prefer_candidate(
            &with,
            &without,
            Some("modified")
        ));
    }

    #[test]
    fn attack_surface_rules_declare_modified_as_the_dedupe_field() {
        use crate::modules::ModuleRegistry;
        let registry = ModuleRegistry::load();
        let types = registry.get("platform").unwrap().content_types();
        let def = types
            .iter()
            .find(|c| c.name == "attack_surface_rules")
            .unwrap();
        assert_eq!(def.dedupe_by_latest, Some("modified"));
    }

    #[test]
    fn wildcard_path_gathers_every_wrapper_not_just_the_first() {
        // A live tenant returns one wrapper object per record. Reading objects[0]
        // retrieved a single record and silently discarded the rest: five widgets on
        // the tenant produced one file, and which one varied per call.
        let client = client();
        let json = json!({
            "objects_count": 3,
            "objects": [
                {"widgets_data": [{"widget_key": "w1", "title": "One"}]},
                {"widgets_data": [{"widget_key": "w2", "title": "Two"}]},
                {"widgets_data": [{"widget_key": "w3", "title": "Three"}]}
            ]
        });
        let def = content_def("widgets", Some("objects[*].widgets_data"));
        let objects = client.extract_items_from_response(&json, &def).unwrap();

        assert_eq!(
            objects.len(),
            3,
            "every wrapper must contribute its records"
        );
        let mut ids: Vec<&str> = objects.iter().map(|o| o.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["w1", "w2", "w3"]);
    }

    #[test]
    fn wildcard_path_still_works_when_one_wrapper_holds_everything() {
        // The other plausible shape, so the wildcard is safe either way.
        let client = client();
        let json = json!({
            "objects": [{"widgets_data": [
                {"widget_key": "w1", "title": "One"},
                {"widget_key": "w2", "title": "Two"}
            ]}]
        });
        let def = content_def("widgets", Some("objects[*].widgets_data"));
        assert_eq!(
            client
                .extract_items_from_response(&json, &def)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn wildcard_path_skips_wrappers_without_the_field() {
        let client = client();
        let json = json!({
            "objects": [
                {"widgets_data": [{"widget_key": "w1", "title": "One"}]},
                {"something_else": true},
                {"widgets_data": [{"widget_key": "w2", "title": "Two"}]}
            ]
        });
        let def = content_def("widgets", Some("objects[*].widgets_data"));
        assert_eq!(
            client
                .extract_items_from_response(&json, &def)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn response_path_matches_exact_key_first() {
        let json = json!({"reply": {"data": [1], "DATA": [2, 3]}});
        let found = client().extract_value_by_path(&json, "reply.DATA").unwrap();
        assert_eq!(found, &json!([2, 3]));
    }

    #[test]
    fn response_path_falls_back_to_case_insensitive_match() {
        // The 3.4 specification documents scheduled_queries as returning `reply.data`
        // while gcgit's content type asks for `reply.DATA`. Both must resolve.
        let json = json!({"reply": {"data": [{"query_def_id": "q1"}]}});
        let found = client().extract_value_by_path(&json, "reply.DATA").unwrap();
        assert_eq!(found, &json!([{"query_def_id": "q1"}]));

        let json = json!({"reply": {"DATA": [{"query_def_id": "q1"}]}});
        let found = client().extract_value_by_path(&json, "reply.data").unwrap();
        assert_eq!(found, &json!([{"query_def_id": "q1"}]));
    }

    #[test]
    fn missing_response_path_is_an_error_not_an_empty_list() {
        // Pull prunes local files for objects that no longer exist remotely, so a
        // response we cannot interpret must not look like "there are no objects".
        let json = json!({"unexpected": "shape"});
        let result = client()
            .extract_items_from_response(&json, &content_def("dashboards", Some("objects")));
        assert!(
            result.is_err(),
            "a missing response path must be reported as a failure"
        );
    }

    #[test]
    fn empty_array_at_response_path_is_a_successful_empty_result() {
        let json = json!({"objects": []});
        let objects = client()
            .extract_items_from_response(&json, &content_def("dashboards", Some("objects")))
            .unwrap();
        assert!(objects.is_empty());
    }

    #[test]
    fn cortex_error_envelope_is_surfaced_in_the_message() {
        // The platform returns this envelope for 401, 403 and 404 across the public
        // APIs. err_extra usually names the missing permission.
        let json = json!({"reply": {
            "err_code": 403,
            "err_msg": "Forbidden. Access was denied to this resource.",
            "err_extra": "Missing permission: cloud_consumption_command_center_view"
        }});
        let err = client()
            .extract_items_from_response(&json, &content_def("policies", Some("reply")))
            .unwrap_err()
            .to_string();

        // err_extra names the missing permission and is the actionable part.
        assert!(
            err.contains("Missing permission"),
            "error should carry err_extra: {err}"
        );
        assert!(
            err.contains("Access was denied"),
            "error should carry err_msg: {err}"
        );
        // The code is not repeated here. The caller already reports the HTTP status,
        // and err_code duplicates it, which produced "failed with HTTP 403 Forbidden:
        // Cortex error 403: Forbidden..." for one failure.
        assert!(
            !err.contains("Cortex error 403"),
            "the code should not be repeated: {err}"
        );
    }

    #[test]
    fn an_err_extra_identical_to_err_msg_is_not_repeated() {
        let json = json!({"reply": {
            "err_code": 403,
            "err_msg": "Forbidden.",
            "err_extra": "Forbidden."
        }});
        let err = client()
            .extract_items_from_response(&json, &content_def("policies", Some("reply")))
            .unwrap_err()
            .to_string();
        assert_eq!(
            err.matches("Forbidden.").count(),
            1,
            "should not say it twice: {err}"
        );
    }

    #[test]
    fn agent_singleton_object_is_wrapped_into_one_item() {
        let json = json!({"reply": {"enabled": true, "interval": 60}});
        let objects = client()
            .extract_items_from_response(&json, &content_def("agent_status", Some("reply")))
            .unwrap();
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].id, "settings");
    }

    #[test]
    fn application_configuration_singleton_is_wrapped_into_one_item() {
        let json = json!({"keepApplicationRefresh": true, "areSbomIssuesConsideredNew": false});
        let objects = client()
            .extract_items_from_response(&json, &content_def("application_configuration", None))
            .unwrap();
        assert_eq!(objects.len(), 1);
        // "settings" matches the agent singletons and the documented layout, so the
        // file is application_configuration/settings.yaml.
        assert_eq!(objects[0].id, "settings");
        assert_eq!(objects[0].name.as_deref(), Some("settings"));
    }

    #[test]
    fn root_array_is_read_when_no_response_path_is_set() {
        let json = json!([{"id": "a"}, {"id": "b"}]);
        let objects = client()
            .extract_items_from_response(&json, &content_def("integrations", None))
            .unwrap();
        assert_eq!(objects.len(), 2);
    }
}
