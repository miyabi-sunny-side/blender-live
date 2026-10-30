//! Animation render jobs: frames, then the MP4 encode.
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Rendering,
    Encoding,
    Done,
    Cancelled,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JobError {
    EmptyRange,
    Busy(String),
    NotFound,
    NotActive,
    WrongPhase(Phase),
    Progress { done: u32, total: u32 },
}

impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRange => f.write_str("frame_end must not be before frame_start"),
            Self::Busy(id) => write!(f, "job {id} is still rendering; wait or cancel it"),
            Self::NotFound => f.write_str("no such job"),
            Self::NotActive => f.write_str("the job has already finished"),
            Self::WrongPhase(phase) => write!(f, "the job is {phase:?}"),
            Self::Progress { done, total } => write!(f, "progress {done}/{total} is invalid"),
        }
    }
}

impl std::error::Error for JobError {}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub created_ms: i64,
    pub frame_start: i32,
    pub frame_end: i32,
    pub done_frames: u32,
    pub phase: Phase,
    pub cancel_requested: bool,
    pub error: Option<String>,
    /// Base camp paths of the MP4 and the frame directory.
    pub outputs: Vec<String>,
}

impl Job {
    pub fn new(
        id: String,
        name: String,
        frame_start: i32,
        frame_end: i32,
        created_ms: i64,
    ) -> Result<Self, JobError> {
        if frame_end < frame_start {
            return Err(JobError::EmptyRange);
        }
        Ok(Self {
            id,
            name,
            created_ms,
            frame_start,
            frame_end,
            done_frames: 0,
            phase: Phase::Rendering,
            cancel_requested: false,
            error: None,
            outputs: Vec::new(),
        })
    }

    #[must_use]
    pub fn total(&self) -> u32 {
        u32::try_from(i64::from(self.frame_end) - i64::from(self.frame_start) + 1)
            .unwrap_or(u32::MAX)
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self.phase, Phase::Rendering | Phase::Encoding)
    }

    pub fn frame_rendered(&mut self, done: u32) -> Result<(), JobError> {
        self.expect(Phase::Rendering)?;
        let total = self.total();
        if done > total || done < self.done_frames {
            return Err(JobError::Progress { done, total });
        }
        self.done_frames = done;
        Ok(())
    }

    pub fn request_cancel(&mut self) -> Result<(), JobError> {
        self.expect_active()?;
        self.cancel_requested = true;
        Ok(())
    }

    pub fn frames_finished(&mut self) -> Result<(), JobError> {
        self.expect(Phase::Rendering)?;
        self.phase = Phase::Encoding;
        Ok(())
    }

    pub fn encoded(&mut self, outputs: Vec<String>) -> Result<(), JobError> {
        self.expect(Phase::Encoding)?;
        self.phase = Phase::Done;
        self.outputs = outputs;
        Ok(())
    }

    /// `kept` lists what the job left behind, such as frames already written.
    pub fn cancelled(&mut self, kept: Vec<String>) -> Result<(), JobError> {
        self.expect_active()?;
        self.phase = Phase::Cancelled;
        self.outputs = kept;
        Ok(())
    }

    pub fn failed(&mut self, error: String) -> Result<(), JobError> {
        self.expect_active()?;
        self.phase = Phase::Failed;
        self.error = Some(error);
        Ok(())
    }

    fn expect(&self, phase: Phase) -> Result<(), JobError> {
        if self.phase == phase {
            Ok(())
        } else {
            Err(JobError::WrongPhase(self.phase))
        }
    }

    fn expect_active(&self) -> Result<(), JobError> {
        if self.is_active() {
            Ok(())
        } else {
            Err(JobError::NotActive)
        }
    }
}

/// All jobs of this process, at most one active at a time.
#[derive(Debug, Default)]
pub struct JobBook {
    jobs: Vec<Job>,
}

impl JobBook {
    pub fn start(&mut self, job: Job) -> Result<(), JobError> {
        if let Some(active) = self.active() {
            return Err(JobError::Busy(active.id.clone()));
        }
        self.jobs.push(job);
        Ok(())
    }

    #[must_use]
    pub fn active(&self) -> Option<&Job> {
        self.jobs.iter().find(|job| job.is_active())
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Job> {
        self.jobs.iter().find(|job| job.id == id)
    }

    pub fn update<T>(
        &mut self,
        id: &str,
        f: impl FnOnce(&mut Job) -> Result<T, JobError>,
    ) -> Result<T, JobError> {
        f(self
            .jobs
            .iter_mut()
            .find(|job| job.id == id)
            .ok_or(JobError::NotFound)?)
    }

    /// Newest first.
    #[must_use]
    pub fn list(&self) -> Vec<Job> {
        self.jobs.iter().rev().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(id: &str, start: i32, end: i32) -> Job {
        Job::new(id.into(), format!("clip-{id}"), start, end, 0).unwrap()
    }

    #[test]
    fn new_jobs_start_rendering_the_inclusive_range() {
        let job = job("a", 1, 48);
        assert_eq!(job.total(), 48);
        assert_eq!(job.phase, Phase::Rendering);
        assert!(job.is_active(), "new job inactive");
        assert_eq!(job.done_frames, 0);
        assert_eq!(
            super::Job::new("b".into(), "b".into(), 5, 5, 0)
                .unwrap()
                .total(),
            1
        );
    }

    #[test]
    fn reversed_ranges_are_refused() {
        assert_eq!(
            Job::new("a".into(), "a".into(), 10, 9, 0),
            Err(JobError::EmptyRange)
        );
    }

    #[test]
    fn progress_moves_forward_within_the_range() {
        let mut job = job("a", 1, 4);
        job.frame_rendered(1).unwrap();
        job.frame_rendered(4).unwrap();
        assert_eq!(job.done_frames, 4);
        assert_eq!(
            job.frame_rendered(5),
            Err(JobError::Progress { done: 5, total: 4 })
        );
        assert_eq!(
            job.frame_rendered(3),
            Err(JobError::Progress { done: 3, total: 4 })
        );
    }

    #[test]
    fn a_finished_render_encodes_then_completes() {
        let mut job = job("a", 1, 2);
        job.frame_rendered(2).unwrap();
        job.frames_finished().unwrap();
        assert_eq!(job.phase, Phase::Encoding);
        assert!(job.is_active(), "encoding is still active");
        assert_eq!(
            job.frame_rendered(2),
            Err(JobError::WrongPhase(Phase::Encoding))
        );
        job.encoded(vec!["renders/clip.mp4".into()]).unwrap();
        assert_eq!(job.phase, Phase::Done);
        assert_eq!(job.outputs, ["renders/clip.mp4"]);
        assert!(!job.is_active(), "done job active");
    }

    #[test]
    fn finished_jobs_refuse_further_transitions() {
        let mut job = job("a", 1, 2);
        job.failed("boom".into()).unwrap();
        assert_eq!(job.phase, Phase::Failed);
        assert_eq!(job.error.as_deref(), Some("boom"));
        assert_eq!(job.request_cancel(), Err(JobError::NotActive));
        assert_eq!(job.cancelled(vec![]), Err(JobError::NotActive));
        assert_eq!(
            job.frames_finished(),
            Err(JobError::WrongPhase(Phase::Failed))
        );
        assert_eq!(
            job.encoded(vec![]),
            Err(JobError::WrongPhase(Phase::Failed))
        );
    }

    #[test]
    fn cancel_is_requested_then_confirmed() {
        let mut job = job("a", 1, 48);
        job.frame_rendered(3).unwrap();
        job.request_cancel().unwrap();
        assert!(job.cancel_requested, "request not recorded");
        assert!(job.is_active(), "cancel waits for the bridge");
        job.cancelled(vec!["renders/clip".into()]).unwrap();
        assert_eq!(job.phase, Phase::Cancelled);
        assert_eq!(job.done_frames, 3);
        assert_eq!(job.outputs, ["renders/clip"], "kept frames not listed");
    }

    #[test]
    fn the_book_allows_one_active_job_and_lists_newest_first() {
        let mut book = JobBook::default();
        book.start(job("a", 1, 2)).unwrap();
        assert_eq!(book.active().map(|j| j.id.as_str()), Some("a"));
        assert_eq!(book.start(job("b", 1, 2)), Err(JobError::Busy("a".into())));
        book.update("a", |j| j.cancelled(vec![])).unwrap();
        assert_eq!(book.active(), None);
        book.start(job("b", 1, 2)).unwrap();
        let ids: Vec<_> = book.list().into_iter().map(|j| j.id).collect();
        assert_eq!(ids, ["b", "a"]);
        assert_eq!(book.get("b").map(|j| j.phase), Some(Phase::Rendering));
        assert_eq!(
            book.update("zz", |j| j.cancelled(vec![])),
            Err(JobError::NotFound)
        );
    }
}
