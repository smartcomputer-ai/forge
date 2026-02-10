use async_trait::async_trait;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanQuestionType {
    YesNo,
    MultipleChoice,
    FreeText,
    Confirmation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanChoice {
    pub key: String,
    pub label: String,
    pub to_node: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HumanQuestion {
    pub stage: String,
    pub text: String,
    pub question_type: HumanQuestionType,
    pub choices: Vec<HumanChoice>,
    pub default_choice: Option<String>,
    pub timeout: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HumanAnswer {
    Selected(String),
    Yes,
    No,
    FreeText(String),
    Timeout,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedInterview {
    pub question: HumanQuestion,
    pub answer: HumanAnswer,
}

#[async_trait]
pub trait Interviewer: Send + Sync {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer;

    async fn ask_multiple(&self, questions: Vec<HumanQuestion>) -> Vec<HumanAnswer> {
        let mut answers = Vec::with_capacity(questions.len());
        for question in questions {
            answers.push(self.ask(question).await);
        }
        answers
    }

    async fn inform(&self, _message: &str, _stage: &str) {}
}

#[derive(Debug, Default)]
pub struct AutoApproveInterviewer;

#[async_trait]
impl Interviewer for AutoApproveInterviewer {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer {
        match question.question_type {
            HumanQuestionType::YesNo | HumanQuestionType::Confirmation => HumanAnswer::Yes,
            HumanQuestionType::MultipleChoice => question
                .choices
                .first()
                .map(|choice| HumanAnswer::Selected(choice.key.clone()))
                .unwrap_or(HumanAnswer::Skipped),
            HumanQuestionType::FreeText => HumanAnswer::FreeText("auto-approved".to_string()),
        }
    }
}

#[derive(Debug, Default)]
pub struct ConsoleInterviewer;

#[async_trait]
impl Interviewer for ConsoleInterviewer {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer {
        let task_question = question.clone();
        match tokio::task::spawn_blocking(move || ask_console(task_question)).await {
            Ok(answer) => answer,
            Err(_) => HumanAnswer::Skipped,
        }
    }
}

pub struct CallbackInterviewer {
    callback: Arc<dyn Fn(HumanQuestion) -> HumanAnswer + Send + Sync>,
}

impl CallbackInterviewer {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(HumanQuestion) -> HumanAnswer + Send + Sync + 'static,
    {
        Self {
            callback: Arc::new(callback),
        }
    }
}

#[async_trait]
impl Interviewer for CallbackInterviewer {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer {
        (self.callback)(question)
    }
}

#[derive(Default)]
pub struct QueueInterviewer {
    answers: Mutex<VecDeque<HumanAnswer>>,
}

impl QueueInterviewer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_answers<I>(answers: I) -> Self
    where
        I: IntoIterator<Item = HumanAnswer>,
    {
        Self {
            answers: Mutex::new(answers.into_iter().collect()),
        }
    }

    pub fn push_answer(&self, answer: HumanAnswer) {
        self.answers
            .lock()
            .expect("queue interviewer mutex should lock")
            .push_back(answer);
    }

    pub fn pending(&self) -> usize {
        self.answers
            .lock()
            .expect("queue interviewer mutex should lock")
            .len()
    }
}

#[async_trait]
impl Interviewer for QueueInterviewer {
    async fn ask(&self, _question: HumanQuestion) -> HumanAnswer {
        self.answers
            .lock()
            .expect("queue interviewer mutex should lock")
            .pop_front()
            .unwrap_or(HumanAnswer::Skipped)
    }
}

pub struct RecordingInterviewer {
    inner: Arc<dyn Interviewer>,
    records: Mutex<Vec<RecordedInterview>>,
}

impl RecordingInterviewer {
    pub fn new(inner: Arc<dyn Interviewer>) -> Self {
        Self {
            inner,
            records: Mutex::new(Vec::new()),
        }
    }

    pub fn recordings(&self) -> Vec<RecordedInterview> {
        self.records
            .lock()
            .expect("recording interviewer mutex should lock")
            .clone()
    }
}

#[async_trait]
impl Interviewer for RecordingInterviewer {
    async fn ask(&self, question: HumanQuestion) -> HumanAnswer {
        let answer = self.inner.ask(question.clone()).await;
        self.records
            .lock()
            .expect("recording interviewer mutex should lock")
            .push(RecordedInterview {
                question,
                answer: answer.clone(),
            });
        answer
    }
}

fn ask_console(question: HumanQuestion) -> HumanAnswer {
    eprintln!("[?] {}", question.text);
    match question.question_type {
        HumanQuestionType::MultipleChoice => {
            for choice in &question.choices {
                eprintln!("  [{}] {}", choice.key, choice.label);
            }
            let raw = match read_line("Select: ") {
                Some(value) => value,
                None => return HumanAnswer::Skipped,
            };
            if raw.is_empty() {
                if let Some(default_choice) = question.default_choice {
                    return HumanAnswer::Selected(default_choice);
                }
                return HumanAnswer::Skipped;
            }
            if let Some(choice_key) = match_choice_key(&question.choices, &raw) {
                return HumanAnswer::Selected(choice_key);
            }
            HumanAnswer::Selected(raw)
        }
        HumanQuestionType::YesNo | HumanQuestionType::Confirmation => {
            let raw = match read_line("[Y/N]: ") {
                Some(value) => value,
                None => return HumanAnswer::Skipped,
            };
            let lowered = raw.trim().to_ascii_lowercase();
            if lowered == "y" || lowered == "yes" {
                HumanAnswer::Yes
            } else if lowered == "n" || lowered == "no" {
                HumanAnswer::No
            } else {
                HumanAnswer::Skipped
            }
        }
        HumanQuestionType::FreeText => {
            let raw = match read_line("> ") {
                Some(value) => value,
                None => return HumanAnswer::Skipped,
            };
            HumanAnswer::FreeText(raw)
        }
    }
}

fn read_line(prompt: &str) -> Option<String> {
    let mut stdout = io::stdout();
    write!(stdout, "{prompt}").ok()?;
    stdout.flush().ok()?;

    let mut raw = String::new();
    io::stdin().read_line(&mut raw).ok()?;
    Some(raw.trim().to_string())
}

fn match_choice_key(choices: &[HumanChoice], raw: &str) -> Option<String> {
    let needle = raw.trim().to_ascii_lowercase();
    choices
        .iter()
        .find(|choice| {
            choice.key.to_ascii_lowercase() == needle
                || choice.label.to_ascii_lowercase() == needle
                || choice.to_node.to_ascii_lowercase() == needle
        })
        .map(|choice| choice.key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn auto_approve_interviewer_multiple_choice_expected_first_selected() {
        let interviewer = AutoApproveInterviewer;
        let answer = interviewer
            .ask(HumanQuestion {
                stage: "gate".to_string(),
                text: "Pick".to_string(),
                question_type: HumanQuestionType::MultipleChoice,
                choices: vec![
                    HumanChoice {
                        key: "A".to_string(),
                        label: "Approve".to_string(),
                        to_node: "ship".to_string(),
                    },
                    HumanChoice {
                        key: "R".to_string(),
                        label: "Revise".to_string(),
                        to_node: "fix".to_string(),
                    },
                ],
                default_choice: None,
                timeout: None,
            })
            .await;
        assert_eq!(answer, HumanAnswer::Selected("A".to_string()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn queue_interviewer_pop_order_expected_fifo_and_then_skipped() {
        let interviewer = QueueInterviewer::with_answers(vec![
            HumanAnswer::Selected("A".to_string()),
            HumanAnswer::Selected("B".to_string()),
        ]);
        let question = HumanQuestion {
            stage: "gate".to_string(),
            text: "Pick".to_string(),
            question_type: HumanQuestionType::MultipleChoice,
            choices: Vec::new(),
            default_choice: None,
            timeout: None,
        };

        assert_eq!(
            interviewer.ask(question.clone()).await,
            HumanAnswer::Selected("A".to_string())
        );
        assert_eq!(
            interviewer.ask(question.clone()).await,
            HumanAnswer::Selected("B".to_string())
        );
        assert_eq!(interviewer.ask(question).await, HumanAnswer::Skipped);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn recording_interviewer_wraps_inner_expected_question_answer_recorded() {
        let inner = Arc::new(QueueInterviewer::with_answers(vec![HumanAnswer::Yes]));
        let recording = RecordingInterviewer::new(inner);
        let question = HumanQuestion {
            stage: "review".to_string(),
            text: "Ship?".to_string(),
            question_type: HumanQuestionType::YesNo,
            choices: Vec::new(),
            default_choice: None,
            timeout: None,
        };

        let answer = recording.ask(question.clone()).await;
        assert_eq!(answer, HumanAnswer::Yes);
        let records = recording.recordings();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].question, question);
        assert_eq!(records[0].answer, HumanAnswer::Yes);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn callback_interviewer_delegate_expected_callback_result() {
        let interviewer = CallbackInterviewer::new(|question| {
            if question.stage == "gate" {
                HumanAnswer::Selected("R".to_string())
            } else {
                HumanAnswer::Skipped
            }
        });

        let answer = interviewer
            .ask(HumanQuestion {
                stage: "gate".to_string(),
                text: "Pick".to_string(),
                question_type: HumanQuestionType::MultipleChoice,
                choices: Vec::new(),
                default_choice: None,
                timeout: None,
            })
            .await;
        assert_eq!(answer, HumanAnswer::Selected("R".to_string()));
    }
}
