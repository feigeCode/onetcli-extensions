mod blob;
mod event;
mod job;

use std::{collections::HashMap, sync::atomic::AtomicI64};

use crate::client::ElasticsearchResource;
use uuid::Uuid;

pub(crate) struct ProviderState {
    resources: HashMap<String, ElasticsearchResource>,
    blobs: HashMap<String, blob::ProviderBlob>,
    jobs: HashMap<String, job::ProviderJob>,
    event_streams: HashMap<String, event::ProviderEventStream>,
    next_reverse_request_id: AtomicI64,
}

impl ProviderState {
    pub(crate) fn new() -> Self {
        Self {
            resources: HashMap::new(),
            blobs: HashMap::new(),
            jobs: HashMap::new(),
            event_streams: HashMap::new(),
            next_reverse_request_id: AtomicI64::new(1),
        }
    }

    pub(crate) fn resource(&self, resource_id: &str) -> Option<&ElasticsearchResource> {
        self.resources.get(resource_id)
    }

    pub(crate) fn cloned_resource(&self, resource_id: &str) -> Option<ElasticsearchResource> {
        self.resource(resource_id).cloned()
    }

    pub(crate) fn insert_resource(&mut self, resource: ElasticsearchResource) -> String {
        let resource_id = loop {
            let candidate = Uuid::new_v4().to_string();
            if !self.resources.contains_key(&candidate) {
                break candidate;
            }
        };
        self.resources.insert(resource_id.clone(), resource);
        resource_id
    }

    pub(crate) fn close_resource(&mut self, resource_id: &str) -> bool {
        if self.resources.remove(resource_id).is_none() {
            return false;
        }
        self.close_jobs_for_resource(resource_id);
        self.close_blobs_for_resource(resource_id);
        true
    }

    pub(crate) fn reverse_request_id(&self) -> &AtomicI64 {
        &self.next_reverse_request_id
    }

    pub(crate) fn clear(&mut self) {
        self.resources.clear();
        self.blobs.clear();
        self.jobs.clear();
        self.event_streams.clear();
    }
}
