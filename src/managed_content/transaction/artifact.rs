use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempArtifact {
    candidate: TempCandidate,
    owner_token: String,
}

impl TempArtifact {
    pub fn new(
        candidate: TempCandidate,
        owner_token: impl Into<String>,
    ) -> Result<Self, CandidateError> {
        let owner_token = owner_token.into();
        if owner_token.is_empty() {
            return Err(CandidateError::EmptyOwnerToken);
        }
        Ok(Self {
            candidate,
            owner_token,
        })
    }

    pub fn candidate(&self) -> &TempCandidate {
        &self.candidate
    }

    pub fn owner_token(&self) -> &str {
        &self.owner_token
    }
}
