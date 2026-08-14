use super::{DomainError, IndexState, RefName, RevisionId, WorktreeState, validate_text};
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum HeadState {
    Unborn,
    Detached { commit: RevisionId },
    Attached { branch: RefName, commit: RevisionId },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum UpstreamState {
    Absent,
    Configured {
        remote: String,
        reference: RefName,
        commit: RevisionId,
    },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitFacts {
    head: HeadState,
    upstream: UpstreamState,
    index: IndexState,
    worktree: WorktreeState,
}

impl GitFacts {
    pub fn new(
        head: HeadState,
        upstream: UpstreamState,
        index: IndexState,
        worktree: WorktreeState,
    ) -> Result<Self, DomainError> {
        if let UpstreamState::Configured { remote, .. } = &upstream {
            validate_text(remote, "upstream remote")?;
        }
        Ok(Self {
            head,
            upstream,
            index: index.normalize()?,
            worktree: worktree.normalize()?,
        })
    }

    pub fn head(&self) -> &HeadState {
        &self.head
    }

    pub fn upstream(&self) -> &UpstreamState {
        &self.upstream
    }

    pub fn index(&self) -> &IndexState {
        &self.index
    }

    pub fn worktree(&self) -> &WorktreeState {
        &self.worktree
    }
}
