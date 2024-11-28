use std::{env, error::Error, fmt, fs};
use serde::Deserialize;
use regex::Regex;
use inline_colorization::*;
use csv::Writer;
use chrono::Local;

use async_openai::{types::{ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs}, Client};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv::dotenv().ok();
    let tests_dir = env::var("TEST_DIR")?;
    match fs::read_dir(tests_dir) {
        Ok(test_files) => {
            let mut writer = Writer::from_path(format!("results{}.csv", Local::now().format("%Y-%m-%d %H%M")))?;
            writer.write_record(&["Name", "Status", "Input", "Result", "Error Location", "Error"])?;
            for path in test_files.map(|p| { p.unwrap() }).filter(|p| { p.file_type().unwrap().is_file() }) {
                let name = path.file_name().to_str().unwrap_or("").to_string();
                let contents = fs::read_to_string(path.path())?;
                match process(&contents).await? {
                    Ok(p) => {
                        println!("Test {} passed", name);
                        writer.write_record(&[name, "Passed".to_string(), contents, p.content, "".to_string(), "".to_string()])?;
                    }
                    Err(e) => {
                        println!("{color_red}Test {} failed.", name);
                        println!("Process: {}", e.location);
                        if let Some(m) = &e.err {
                            println!("{}", m)
                        }
                        print!("{color_reset}");
                        writer.write_record(&[name, "Failed".to_string(), contents, e.content, e.location.to_string(), e.err.unwrap_or("".to_string())])?;
                    }
                }
            }
            writer.flush()?;
        }
        Err(e) => panic!("{}", e)
    }
    Ok(())
}

async fn process(contents: &String) -> Result<Result<TestPass, TestError>, Box<dyn Error>> {
    let input_r = Regex::new(r"(?s)<input>(.*?)</input>")?;
    let output_r = Regex::new(r"(?s)<output>(.*?)</output>")?;
    let input =
        if let Some(m) = input_r.captures(contents) {
            m.get(1).unwrap().as_str()
        } else {
            return Ok(Err(TestError { content: contents.to_string(), location: ErrorLocation::MatchInput, err: None }));
        };
    let expected_output =
        if let Some(m) = output_r.captures(contents) {
            m.get(1).unwrap().as_str()
        } else {
            return Ok(Err(TestError { content: contents.to_string(), location: ErrorLocation::MatchInput, err: None }));
        };
    let gen_prompt = fs::read_to_string(env::var("GEN_PROMPT")?)?;
    let test_prompt = fs::read_to_string(env::var("TEST_PROMPT")?)?;
    let model = env::var("model")?;
    let client = Client::new();
    let req = CreateChatCompletionRequestArgs::default()
        .model(&model)
        .messages([
            ChatCompletionRequestUserMessageArgs::default()
            .content(gen_prompt.replace("__description__", &input))
            .build()?.into()
        ])
        .build()?;
    let res = client.chat().create(req).await?;
    let message = res.choices.first().unwrap().message.content.clone().unwrap();
    let r = Regex::new(r"(\{(.|\n)*?\}|\[(.|\n)*?\])")?;
    if let Some(m) = r.find(&message) {
        let jzml = m.as_str();
        Ok(match serde_json::from_str::<DescriptionInfo>(jzml) {
            Ok(info) => {
                let req = CreateChatCompletionRequestArgs::default()
                    .model(model)
                    .messages([
                        ChatCompletionRequestUserMessageArgs::default()
                    .content(test_prompt
                        .replace("__description__", input)
                        .replace("__baseline__", expected_output)
                        .replace("__input__", jzml))
                    .build()?.into()
                ])
                .build()?;
                let res = client.chat().create(req).await?;
                let test_message = res.choices.first().unwrap().message.content.clone().unwrap();
                if test_message.to_lowercase() == "true" {
                    Ok(TestPass { content: message, info })
                } else {
                    Err(TestError { content: message, location: ErrorLocation::Test, err: None })
                }
            },
            Err(e) => Err(TestError { content: message, location: ErrorLocation::Parse, err: Some(e.to_string()) })
        })
    } else {
        Ok(Err(TestError { content: message, location: ErrorLocation::MatchJson, err: None }))
    }
}

#[derive(Debug)]
struct TestInfo {
    input: String,
    expected: String
}

#[derive(Debug)]
struct TestPass {
    content: String,
    info: DescriptionInfo
}

#[derive(Deserialize, Debug)]
struct DescriptionInfo {
    problem: String,
    resolution: String
}

#[derive(Debug)]
enum ErrorLocation {
    MatchInput,
    MatchJson,
    Parse,
    Test
}

impl fmt::Display for ErrorLocation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", match self {
            ErrorLocation::MatchInput => "matchinput",
            ErrorLocation::MatchJson => "matchjson",
            ErrorLocation::Parse => "parse",
            ErrorLocation::Test => "test"
        })
    }
}

#[derive(Debug)]
struct TestError {
    content: String,
    location: ErrorLocation,
    err: Option<String>
}
