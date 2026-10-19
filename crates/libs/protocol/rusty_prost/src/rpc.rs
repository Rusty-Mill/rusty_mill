//! `google.rpc.Status` and `google.rpc.ErrorInfo`, the error detail gRPC carries in
//! `grpc-status-details-bin`.

use std::collections::BTreeMap;

use crate::wkt::Any;
use crate::{field, Field, Message, Name, Result, Writer};

/// `google.rpc.Status`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Status {
    /// A `google.rpc.Code` value.
    pub code: i32,
    /// A developer-facing message.
    pub message: String,
    /// Typed details, usually an [`ErrorInfo`] packed in an [`Any`].
    pub details: Vec<Any>,
}

/// `google.rpc.ErrorInfo`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ErrorInfo {
    /// A constant, machine-readable reason.
    pub reason: String,
    /// The logical grouping the reason belongs to.
    pub domain: String,
    /// Additional structured details.
    pub metadata: BTreeMap<String, String>,
}

impl Name for Status {
    const TYPE_URL: &'static str = "type.googleapis.com/google.rpc.Status";
}

impl Name for ErrorInfo {
    const TYPE_URL: &'static str = "type.googleapis.com/google.rpc.ErrorInfo";
}

impl Message for Status {
    fn encode(&self, w: &mut Writer) {
        if self.code != 0 {
            w.int32(1, self.code);
        }
        if !self.message.is_empty() {
            w.string(2, &self.message);
        }
        for d in &self.details {
            w.message(3, |w| d.encode(w));
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.code = f.int32()?,
            2 => self.message = f.str()?.to_owned(),
            3 => field::push_message(&mut self.details, &f)?,
            _ => {}
        }
        Ok(())
    }
}

impl Message for ErrorInfo {
    fn encode(&self, w: &mut Writer) {
        if !self.reason.is_empty() {
            w.string(1, &self.reason);
        }
        if !self.domain.is_empty() {
            w.string(2, &self.domain);
        }
        for (k, v) in &self.metadata {
            w.message(3, |w| {
                w.string(1, k);
                w.string(2, v);
            });
        }
    }

    fn merge_field(&mut self, f: Field<'_>) -> Result<()> {
        match f.number {
            1 => self.reason = f.str()?.to_owned(),
            2 => self.domain = f.str()?.to_owned(),
            3 => field::map_entry(
                &mut self.metadata,
                &f,
                |k, f| {
                    *k = f.str()?.to_owned();
                    Ok(())
                },
                |v, f| {
                    *v = f.str()?.to_owned();
                    Ok(())
                },
            )?,
            _ => {}
        }
        Ok(())
    }
}
