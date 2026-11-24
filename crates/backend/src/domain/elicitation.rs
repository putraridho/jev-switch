//! A server's request for information from the person, independent of tool permissions.

/// A question waiting on the provider's control channel. Its contents may be private.
#[derive(Clone, PartialEq)]
pub struct ElicitationRequest {
    pub id: String,
    pub server: String,
    pub message: String,
    pub kind: ElicitationKind,
}

impl std::fmt::Debug for ElicitationRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ElicitationRequest").field("id", &self.id).finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationKind {
    Form(Vec<ElicitationField>),
    Url(String),
    Unsupported(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElicitationField {
    pub name: String,
    pub title: String,
    pub description: String,
    pub required: bool,
    pub kind: FieldKind,
    pub default: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text,
    Choice(Vec<String>),
    Boolean,
    Integer,
    Number,
}

/// Typed form contents; the adapter owns converting them to the provider's JSON.
#[derive(Debug, Clone, PartialEq)]
pub enum ElicitationValue {
    Text(String),
    Boolean(bool),
    Integer(i64),
    Number(f64),
}

/// No persistent grant or auto-approval exists for a question.
#[derive(Clone, PartialEq)]
pub enum ElicitationAnswer {
    Accept(Vec<(String, ElicitationValue)>),
    Decline,
    Cancel,
}

impl std::fmt::Debug for ElicitationAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self { Self::Accept(_) => "Accept(..)", Self::Decline => "Decline", Self::Cancel => "Cancel" })
    }
}

impl ElicitationField {
    /// Empty optional fields are omitted. An answer must satisfy the field's scalar type
    /// and bounds; the server remains responsible for additional format validation.
    pub fn parse(&self, text: &str) -> Result<Option<ElicitationValue>, String> {
        if text.is_empty() && !self.required { return Ok(None); }
        if text.is_empty() && !matches!(self.kind, FieldKind::Text | FieldKind::Choice(_)) {
            return Err("This field is required".into());
        }
        let value = match &self.kind {
            FieldKind::Text | FieldKind::Choice(_) => {
                let len = text.chars().count();
                if self.min_length.is_some_and(|n| len < n) || self.max_length.is_some_and(|n| len > n) {
                    return Err("Text length is outside the requested limits".into());
                }
                if let FieldKind::Choice(choices) = &self.kind {
                    if !choices.iter().any(|s| s == text) { return Err("Choose one of the listed values".into()); }
                }
                ElicitationValue::Text(text.into())
            }
            FieldKind::Boolean => match text.to_ascii_lowercase().as_str() {
                "true" | "yes" => ElicitationValue::Boolean(true),
                "false" | "no" => ElicitationValue::Boolean(false),
                _ => return Err("Enter true or false".into()),
            },
            FieldKind::Integer => ElicitationValue::Integer(text.parse().map_err(|_| "Enter a whole number")?),
            FieldKind::Number => {
                let n: f64 = text.parse().map_err(|_| "Enter a number")?;
                if !n.is_finite() { return Err("Enter a finite number".into()); }
                ElicitationValue::Number(n)
            }
        };
        let number = match &value { ElicitationValue::Number(n) => Some(*n), ElicitationValue::Integer(n) => Some(*n as f64), _ => None };
        if number.is_some_and(|n| self.min.is_some_and(|m| n < m) || self.max.is_some_and(|m| n > m)) {
            return Err("Number is outside the requested limits".into());
        }
        Ok(Some(value))
    }
}
