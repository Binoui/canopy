use crate::{address::WebOrigin, secret::Secret};

#[derive(Debug)]
pub struct Login {
    pub origin: WebOrigin,
    pub username: String,
    pub password: Secret,
    pub last_used: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoginId(i64);

#[derive(Clone, Debug)]
pub struct SavedLogin {
    pub id: LoginId,
    pub origin: WebOrigin,
    pub username: String,
}
