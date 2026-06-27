// Copyright (c) Ankit Chaubey <ankitchaubey.dev@gmail.com>
//
// ferogram: async Telegram MTProto client in Rust
// https://github.com/ankit-chaubey/ferogram
//
// Licensed under either the MIT License or the Apache License 2.0.
// See the LICENSE-MIT or LICENSE-APACHE file in this repository:
// https://github.com/ankit-chaubey/ferogram
//
// Feel free to use, modify, and share this code.
// Please keep this notice when redistributing.

use ferogram_tl_types as tl;

/// Fluent builder for polls sent via [`crate::Client::send_poll`].
///
/// ```rust,no_run
/// # use ferogram::{Client, poll::PollBuilder};
/// # async fn f(client: Client, peer: ferogram_tl_types::enums::Peer) -> Result<(), Box<dyn std::error::Error>> {
/// client.send_poll(peer, PollBuilder::new("Best language?")
///     .answers(["Rust", "Go", "C++"])
///     .public_voters(true)).await?;
/// # Ok(()) }
/// ```
#[derive(Debug, Clone)]
pub struct PollBuilder {
    pub question: String,
    pub answers: Vec<String>,
    pub quiz: bool,
    pub correct_index: Option<usize>,
    pub multiple_choice: bool,
    pub subscribers_only: bool,
    pub countries_iso2: Vec<String>,
    pub close_period: Option<i32>,
    pub close_date: Option<i32>,
    pub public_voters: bool,
    pub shuffle_answers: bool,
    pub hide_results_until_close: bool,
    pub solution: Option<String>,
}

impl PollBuilder {
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            answers: vec![],
            quiz: false,
            correct_index: None,
            multiple_choice: false,
            subscribers_only: false,
            countries_iso2: vec![],
            close_period: None,
            close_date: None,
            public_voters: false,
            shuffle_answers: false,
            hide_results_until_close: false,
            solution: None,
        }
    }

    pub fn answers<I, S>(mut self, answers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.answers = answers.into_iter().map(Into::into).collect();
        self
    }

    pub fn quiz(mut self, v: bool) -> Self {
        self.quiz = v;
        self
    }

    pub fn correct_index(mut self, idx: usize) -> Self {
        self.correct_index = Some(idx);
        self
    }

    pub fn multiple_choice(mut self, v: bool) -> Self {
        self.multiple_choice = v;
        self
    }

    /// Only subscribers can vote.
    pub fn subscribers_only(mut self, v: bool) -> Self {
        self.subscribers_only = v;
        self
    }

    /// Restrict voting to users from these ISO 3166-1 alpha-2 country codes.
    pub fn countries_iso2<I, S>(mut self, codes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.countries_iso2 = codes.into_iter().map(Into::into).collect();
        self
    }

    /// Auto-close after this many seconds (1-600).
    pub fn close_period(mut self, secs: i32) -> Self {
        self.close_period = Some(secs);
        self
    }

    /// Auto-close at this Unix timestamp.
    pub fn close_date(mut self, ts: i32) -> Self {
        self.close_date = Some(ts);
        self
    }

    pub fn public_voters(mut self, v: bool) -> Self {
        self.public_voters = v;
        self
    }

    pub fn shuffle_answers(mut self, v: bool) -> Self {
        self.shuffle_answers = v;
        self
    }

    pub fn hide_results_until_close(mut self, v: bool) -> Self {
        self.hide_results_until_close = v;
        self
    }

    /// Explanation shown after a quiz answer (supports HTML/Markdown entities).
    pub fn solution(mut self, text: impl Into<String>) -> Self {
        self.solution = Some(text.into());
        self
    }

    /// Build the `InputMedia` ready to pass to `SendMedia`.
    pub(crate) fn into_input_media(self) -> tl::enums::InputMedia {
        let poll_answers: Vec<tl::enums::PollAnswer> = self
            .answers
            .iter()
            .enumerate()
            .map(|(i, a)| {
                tl::enums::PollAnswer::PollAnswer(tl::types::PollAnswer {
                    text: tl::enums::TextWithEntities::TextWithEntities(
                        tl::types::TextWithEntities {
                            text: a.clone(),
                            entities: vec![],
                        },
                    ),
                    option: vec![i as u8],
                    media: None,
                    added_by: None,
                    date: None,
                })
            })
            .collect();

        let correct_answers: Option<Vec<i32>> = if self.quiz {
            self.correct_index.map(|i| vec![i as i32])
        } else {
            None
        };

        let poll = tl::enums::Poll::Poll(tl::types::Poll {
            id: 0,
            closed: false,
            public_voters: self.public_voters,
            multiple_choice: self.multiple_choice && !self.quiz,
            quiz: self.quiz,
            open_answers: false,
            revoting_disabled: false,
            shuffle_answers: self.shuffle_answers,
            hide_results_until_close: self.hide_results_until_close,
            creator: false,
            subscribers_only: false,
            question: tl::enums::TextWithEntities::TextWithEntities(tl::types::TextWithEntities {
                text: self.question,
                entities: vec![],
            }),
            answers: poll_answers,
            close_period: self.close_period,
            close_date: self.close_date,
            countries_iso2: None,
            hash: 0,
        });

        let solution_entities = self.solution.as_ref().map(|_| vec![]);
        let solution = self.solution;

        tl::enums::InputMedia::Poll(Box::new(tl::types::InputMediaPoll {
            poll,
            correct_answers,
            attached_media: None,
            solution,
            solution_entities,
            solution_media: None,
        }))
    }
}
