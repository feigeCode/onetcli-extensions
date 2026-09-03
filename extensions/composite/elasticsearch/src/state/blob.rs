use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use extension_protocol::{
    blob::{BlobCloseParams, BlobReadParams, BlobReadResult, should_stream_blob},
    error::{ProtocolError, error_codes},
    resource::ResourceInvokeResult,
    result_ref::ResultRef,
};
use serde_json::Value;
use uuid::Uuid;

use super::ProviderState;
use crate::error::{boxed_error, invalid_params};

const MAX_TOTAL_BLOB_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct ProviderBlob {
    resource_id: String,
    data: Vec<u8>,
    offset: usize,
    closed: bool,
}

impl ProviderState {
    fn total_blob_bytes(&self) -> usize {
        self.blobs.values().map(|blob| blob.data.len()).sum()
    }

    pub(super) fn store_blob(&mut self, resource_id: &str, data: Vec<u8>) -> Option<String> {
        if data.len() > MAX_TOTAL_BLOB_BYTES
            || self.total_blob_bytes().saturating_add(data.len()) > MAX_TOTAL_BLOB_BYTES
        {
            return None;
        }
        let blob_id = format!("es-blob-{}", Uuid::new_v4());
        self.blobs.insert(
            blob_id.clone(),
            ProviderBlob {
                resource_id: resource_id.to_owned(),
                data,
                offset: 0,
                closed: false,
            },
        );
        Some(blob_id)
    }

    pub(crate) fn blob_result(
        &mut self,
        resource_id: &str,
        value: Value,
    ) -> Result<ResourceInvokeResult, Box<ProtocolError>> {
        let data = serde_json::to_vec(&value)
            .map_err(|error| invalid_params(format!("failed to encode result: {error}")))?;
        if !should_stream_blob(data.len() as u64) {
            return Ok(ResourceInvokeResult {
                result: ResultRef::Inline { value },
            });
        }
        let Some(blob_id) = self.store_blob(resource_id, data) else {
            return Err(boxed_error(
                error_codes::DATA_VALUE_OUT_OF_RANGE,
                "Elasticsearch result exceeds the provider blob budget",
            ));
        };
        Ok(ResourceInvokeResult {
            result: ResultRef::Blob { id: blob_id },
        })
    }

    pub(crate) fn read_blob(
        &mut self,
        params: BlobReadParams,
    ) -> Result<BlobReadResult, Box<ProtocolError>> {
        let max_bytes = params.effective_max_bytes() as usize;
        let blob = self.blobs.get_mut(&params.blob_id).ok_or_else(|| {
            boxed_error(error_codes::RESOURCE_CLOSED, "blob is closed or unknown")
        })?;
        if blob.closed {
            return Err(boxed_error(error_codes::RESOURCE_CLOSED, "blob is closed"));
        }
        let start = blob.offset.min(blob.data.len());
        let end = start.saturating_add(max_bytes).min(blob.data.len());
        let bytes_read = end.saturating_sub(start) as u32;
        let done = end == blob.data.len() && bytes_read > 0;
        blob.offset = end;
        Ok(BlobReadResult {
            data: BASE64.encode(&blob.data[start..end]),
            bytes_read,
            done,
        })
    }

    pub(crate) fn close_blob(&mut self, params: BlobCloseParams) {
        self.blobs.remove(&params.blob_id);
    }

    pub(crate) fn close_blobs_for_resource(&mut self, resource_id: &str) {
        self.blobs.retain(|_, blob| blob.resource_id != resource_id);
    }
}
