use extension_protocol::{
    error::{ProtocolError, error_codes},
    job::{
        JobCancelParams, JobCloseParams, JobResultParams, JobResultResult, JobStartParams,
        JobStartResult, JobState, JobStatusParams, JobStatusResult, ProgressPercent,
    },
    result_ref::ResultRef,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use super::ProviderState;
use crate::{
    client::{execute, normalize_search, search_body},
    error::{boxed_error, resource_error},
};

const MAX_JOBS: usize = 64;

#[derive(Debug)]
pub(super) struct ProviderJob {
    resource_id: String,
    pub(super) state: JobState,
    pub(super) progress_percent: Option<ProgressPercent>,
    pub(super) message: Option<String>,
    pub(super) result: Option<JobResultResult>,
    completion: Option<mpsc::Receiver<Result<Value, Box<ProtocolError>>>>,
    cancellation: Option<oneshot::Sender<()>>,
}

impl ProviderState {
    pub(crate) fn start_job(
        &mut self,
        params: JobStartParams,
    ) -> Result<JobStartResult, Box<ProtocolError>> {
        let resource_id = params.resource_id.clone().ok_or_else(resource_error)?;
        let resource = self
            .cloned_resource(&resource_id)
            .ok_or_else(resource_error)?;
        if params.method != "elasticsearch/search/async" {
            return Err(boxed_error(
                error_codes::METHOD_NOT_FOUND,
                format!("unknown Elasticsearch job method `{}`", params.method),
            ));
        }
        if self.jobs.len() >= MAX_JOBS {
            return Err(boxed_error(
                error_codes::RESOURCE_BUSY,
                "Elasticsearch job limit reached",
            ));
        }
        search_body(&params.params)?;
        let job_id = format!("es-job-{}", Uuid::new_v4());
        let (completion_tx, completion_rx) = mpsc::channel(1);
        let (cancel_tx, cancel_rx) = oneshot::channel();
        tokio::spawn(async move {
            tokio::select! {
                result = execute(&resource, "elasticsearch/search", &params.params) => {
                    let _ = completion_tx.send(result).await;
                }
                _ = cancel_rx => {}
            }
        });
        self.jobs.insert(
            job_id.clone(),
            ProviderJob {
                resource_id,
                state: JobState::Running,
                progress_percent: None,
                message: Some("search running".to_owned()),
                result: None,
                completion: Some(completion_rx),
                cancellation: Some(cancel_tx),
            },
        );
        Ok(JobStartResult {
            job_id,
            state: JobState::Running,
        })
    }

    pub(crate) fn poll_job(&mut self, job_id: &str) -> bool {
        let outcome = {
            let Some(job) = self.jobs.get_mut(job_id) else {
                return false;
            };
            if job.state != JobState::Running {
                return false;
            }
            let Some(mut completion) = job.completion.take() else {
                job.state = JobState::Failed;
                job.message = Some("search worker is unavailable".to_owned());
                return true;
            };
            match completion.try_recv() {
                Ok(Ok(value)) => Some(Ok(value)),
                Ok(Err(error)) => Some(Err(error.message)),
                Err(mpsc::error::TryRecvError::Empty) => {
                    job.completion = Some(completion);
                    None
                }
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    Some(Err("search worker terminated".to_owned()))
                }
            }
        };

        let Some(outcome) = outcome else {
            return false;
        };
        match outcome {
            Ok(value) => self.complete_job(job_id, value),
            Err(message) => {
                if let Some(job) = self.jobs.get_mut(job_id) {
                    job.state = JobState::Failed;
                    job.progress_percent = None;
                    job.message = Some(message);
                    job.result = None;
                }
                true
            }
        }
    }

    fn complete_job(&mut self, job_id: &str, value: Value) -> bool {
        let value = normalize_search(value);
        let Some(resource_id) = self.jobs.get(job_id).map(|job| job.resource_id.clone()) else {
            return false;
        };
        let data = match serde_json::to_vec(&value) {
            Ok(data) => data,
            Err(error) => {
                if let Some(job) = self.jobs.get_mut(job_id) {
                    job.state = JobState::Failed;
                    job.message = Some(error.to_string());
                    job.result = None;
                }
                return true;
            }
        };
        let result_ref = if extension_protocol::blob::should_stream_blob(data.len() as u64) {
            match self.store_blob(&resource_id, data) {
                Some(blob_id) => ResultRef::Blob { id: blob_id },
                None => {
                    if let Some(job) = self.jobs.get_mut(job_id) {
                        job.state = JobState::Failed;
                        job.progress_percent = None;
                        job.message = Some(
                            "Elasticsearch job result exceeds the provider blob budget".to_owned(),
                        );
                        job.result = None;
                    }
                    return true;
                }
            }
        } else {
            ResultRef::Inline { value }
        };
        if let Some(job) = self.jobs.get_mut(job_id) {
            job.state = JobState::Succeeded;
            job.progress_percent = Some(ProgressPercent::new(100).expect("valid progress"));
            job.message = Some("search completed".to_owned());
            job.result = Some(JobResultResult { result: result_ref });
        }
        true
    }

    pub(crate) fn job_status(
        &self,
        params: JobStatusParams,
    ) -> Result<JobStatusResult, Box<ProtocolError>> {
        let job = self
            .jobs
            .get(&params.job_id)
            .ok_or_else(|| boxed_error(error_codes::RESOURCE_CLOSED, "job is closed or unknown"))?;
        Ok(JobStatusResult {
            job_id: params.job_id,
            state: job.state,
            progress_percent: job.progress_percent,
            message: job.message.clone(),
        })
    }

    pub(crate) fn cancel_job(
        &mut self,
        params: JobCancelParams,
    ) -> Result<bool, Box<ProtocolError>> {
        let Some(job) = self.jobs.get_mut(&params.job_id) else {
            return Err(boxed_error(
                error_codes::RESOURCE_CLOSED,
                "job is closed or unknown",
            ));
        };
        if job.state == JobState::Running {
            job.state = JobState::Cancelled;
            job.progress_percent = None;
            job.message = Some("search cancelled".to_owned());
            job.result = None;
            job.completion = None;
            if let Some(cancel) = job.cancellation.take() {
                let _ = cancel.send(());
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn job_result(
        &self,
        params: JobResultParams,
    ) -> Result<JobResultResult, Box<ProtocolError>> {
        let job = self
            .jobs
            .get(&params.job_id)
            .ok_or_else(|| boxed_error(error_codes::RESOURCE_CLOSED, "job is closed or unknown"))?;
        match job.state {
            JobState::Succeeded => {}
            JobState::Running => {
                return Err(boxed_error(
                    error_codes::RESOURCE_BUSY,
                    "job result is not ready",
                ));
            }
            JobState::Cancelled => {
                return Err(boxed_error(
                    error_codes::REQUEST_CANCELLED,
                    "job was cancelled",
                ));
            }
            state => {
                return Err(boxed_error(
                    error_codes::INTERNAL_ERROR,
                    format!("job failed with state `{state:?}`"),
                ));
            }
        }
        job.result
            .clone()
            .ok_or_else(|| boxed_error(error_codes::INTERNAL_ERROR, "job result is unavailable"))
    }

    pub(crate) fn close_job(&mut self, params: JobCloseParams) {
        self.close_job_by_id(&params.job_id);
    }

    pub(crate) fn close_jobs_for_resource(&mut self, resource_id: &str) {
        let job_ids = self
            .jobs
            .iter()
            .filter(|(_, job)| job.resource_id == resource_id)
            .map(|(job_id, _)| job_id.clone())
            .collect::<Vec<_>>();
        for job_id in job_ids {
            self.close_job_by_id(&job_id);
        }
    }

    fn close_job_by_id(&mut self, job_id: &str) {
        if let Some(mut job) = self.jobs.remove(job_id) {
            if job.state == JobState::Running
                && let Some(cancel) = job.cancellation.take()
            {
                let _ = cancel.send(());
            }
            if let Some(JobResultResult {
                result: ResultRef::Blob { id },
            }) = job.result
            {
                self.blobs.remove(&id);
            }
        }
    }
}
