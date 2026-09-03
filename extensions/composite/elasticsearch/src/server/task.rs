use extension_protocol::job::{
    JobCancelParams, JobCloseParams, JobResultParams, JobStartParams, JobStatusParams,
};
use serde_json::Value;

use crate::{
    error::{ProviderResult, parse_params, serialize},
    state::ProviderState,
};

pub(super) fn start(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: JobStartParams = parse_params(params)?;
    serialize(state.start_job(params)?)
}

pub(super) fn status(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: JobStatusParams = parse_params(params)?;
    if state.poll_job(&params.job_id) {
        state.emit_job_event(&params.job_id);
    }
    serialize(state.job_status(params)?)
}

pub(super) fn cancel(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: JobCancelParams = parse_params(params)?;
    let job_id = params.job_id.clone();
    if state.cancel_job(params)? {
        state.emit_job_event(&job_id);
    }
    Ok(Value::Null)
}

pub(super) fn result(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: JobResultParams = parse_params(params)?;
    if state.poll_job(&params.job_id) {
        state.emit_job_event(&params.job_id);
    }
    serialize(state.job_result(params)?)
}

pub(super) fn close(state: &mut ProviderState, params: Value) -> ProviderResult {
    let params: JobCloseParams = parse_params(params)?;
    state.close_job(params);
    Ok(Value::Null)
}
