use std::collections::VecDeque;

use extension_protocol::{
    error::{ProtocolError, error_codes},
    event_stream::{
        DEFAULT_EVENT_MAX_EVENTS, EventOpenParams, EventOpenResult, EventReadParams,
        EventReadResult, MAX_EVENT_MAX_EVENTS,
    },
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::ProviderState;
use crate::error::{boxed_error, invalid_params, resource_error};

const MAX_EVENT_STREAMS: usize = 64;
const SEARCH_EVENT_KIND: &str = "elasticsearch/search/events";

#[derive(Debug)]
pub(super) struct ProviderEventStream {
    kind: String,
    buffer: VecDeque<Value>,
    capacity: usize,
    dropped_count: u64,
    closed: bool,
}

impl ProviderState {
    pub(crate) fn open_event_stream(
        &mut self,
        params: EventOpenParams,
    ) -> Result<EventOpenResult, Box<ProtocolError>> {
        if params.kind != SEARCH_EVENT_KIND {
            return Err(boxed_error(
                error_codes::METHOD_NOT_FOUND,
                format!("unknown Elasticsearch event stream `{}`", params.kind),
            ));
        }
        if params.conn_id.is_some() {
            return Err(invalid_params(
                "Elasticsearch event streams are provider-global",
            ));
        }
        if self.resources.is_empty() {
            return Err(resource_error());
        }
        if self.event_streams.len() >= MAX_EVENT_STREAMS {
            return Err(boxed_error(
                error_codes::RESOURCE_BUSY,
                "Elasticsearch event stream limit reached",
            ));
        }
        let stream_id = format!("es-stream-{}", Uuid::new_v4());
        self.event_streams.insert(
            stream_id.clone(),
            ProviderEventStream {
                kind: params.kind,
                buffer: VecDeque::new(),
                capacity: params
                    .capacity
                    .unwrap_or(DEFAULT_EVENT_MAX_EVENTS)
                    .clamp(1, MAX_EVENT_MAX_EVENTS) as usize,
                dropped_count: 0,
                closed: false,
            },
        );
        Ok(EventOpenResult { stream_id })
    }

    fn push_event(&mut self, stream_id: &str, event: Value) {
        let Some(stream) = self.event_streams.get_mut(stream_id) else {
            return;
        };
        if stream.closed {
            return;
        }
        if stream.buffer.len() >= stream.capacity {
            let _ = stream.buffer.pop_front();
            stream.dropped_count = stream.dropped_count.saturating_add(1);
        }
        stream.buffer.push_back(event);
    }

    fn broadcast_event(&mut self, event: Value) {
        let stream_ids = self
            .event_streams
            .iter()
            .filter(|(_, stream)| stream.kind == SEARCH_EVENT_KIND)
            .map(|(stream_id, _)| stream_id.clone())
            .collect::<Vec<_>>();
        for stream_id in stream_ids {
            self.push_event(&stream_id, event.clone());
        }
    }

    pub(crate) fn read_event_stream(&mut self, params: EventReadParams) -> EventReadResult {
        let Some(stream) = self.event_streams.get_mut(&params.stream_id) else {
            return EventReadResult {
                events: Vec::new(),
                closed: true,
                dropped_count: 0,
            };
        };
        let count = params
            .effective_max_events()
            .min(stream.buffer.len() as u32) as usize;
        EventReadResult {
            events: stream.buffer.drain(..count).collect(),
            closed: stream.closed,
            dropped_count: stream.dropped_count,
        }
    }

    pub(crate) fn close_event_stream(&mut self, stream_id: &str) {
        self.event_streams.remove(stream_id);
    }

    pub(crate) fn emit_job_event(&mut self, job_id: &str) {
        let Some(job) = self.jobs.get(job_id) else {
            return;
        };
        let event = json!({
            "type": "job/completed",
            "job_id": job_id,
            "state": job.state,
            "progress_percent": job.progress_percent.map(u8::from),
            "message": job.message,
            "result": job.result,
        });
        self.broadcast_event(event);
    }
}
