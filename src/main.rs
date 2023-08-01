use reqwest::{blocking::Client, StatusCode};
use serde_derive::Deserialize;
use std::env;
use std::io::Read;
use std::time::Duration;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_TITLE_LENGTH: usize = 44;

// Oh shit, it's på norsk!
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct Organization {
    organisasjonsnummer: String,
    navn: String,
    slettedato: Option<String>,
    registreringsdato_enhetsregisteret: Option<String>,
    postadresse: Option<Adresse>,
    forretningsadresse: Option<Adresse>,
    under_avvikling: Option<bool>,
    under_tvangsavvikling_eller_tvangsopplosning: Option<bool>,
    hjemmeside: Option<String>,
    overordnet_enhet: Option<String>,
}
#[derive(Deserialize, Debug)]
struct Adresse {
    adresse: Vec<String>,
    postnummer: Option<String>,
    poststed: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Underenheter {
    underenheter: Vec<Organization>,
}

#[derive(Deserialize, Debug)]
struct SearchResponse {
    _embedded: Option<Underenheter>,
}

// https://data.brreg.no/enhetsregisteret/api/docs/index.html#_500_feil_p%C3%A5_server
#[derive(Deserialize, Debug)]
struct BrregInternalServerError {
    trace: String,
    error: String,
    message: String,
}

#[derive(Debug, PartialEq)]
enum BrregErrorType {
    NotFound,
    Gone,
    InternalServerError,
    NetworkError,
    UnexpectedResponse,
    JsonParseError,
}

#[derive(Debug)]
struct BrregError {
    typ: BrregErrorType,
    error: Option<String>,
}

fn get_organization(client: &Client, orgnr: &str, typ: &str) -> Result<Organization, BrregError> {
    let http_result = client
        .get(format!(
            "https://data.brreg.no/enhetsregisteret/api/{}er/{}",
            typ, orgnr
        ))
        .send();

    if http_result.is_err() {
        return Err(BrregError {
            typ: BrregErrorType::NetworkError,
            error: Some(http_result.unwrap_err().to_string()),
        });
    }

    let mut response = http_result.unwrap();

    let mut body = String::new();
    let read_result = response.read_to_string(&mut body);
    if read_result.is_err() {
        return Err(BrregError {
            typ: BrregErrorType::NetworkError,
            error: Some(read_result.unwrap_err().to_string()),
        });
    }

    match response.status() {
        StatusCode::OK => {
            let json_parse_res = serde_json::from_str::<Organization>(&body);
            if json_parse_res.is_ok() {
                return Ok(json_parse_res.unwrap());
            }
            return Err(BrregError {
                typ: BrregErrorType::JsonParseError,
                error: Some(json_parse_res.unwrap_err().to_string()),
            });
        }

        StatusCode::NOT_FOUND => Err(BrregError {
            typ: BrregErrorType::NotFound,
            error: None,
        }),

        StatusCode::GONE => Err(BrregError {
            typ: BrregErrorType::Gone,
            error: None,
        }),

        StatusCode::INTERNAL_SERVER_ERROR => {
            let json_parse_res = serde_json::from_str::<BrregInternalServerError>(&body);
            if json_parse_res.is_ok() {
                let brreg_error = json_parse_res.unwrap();

                return Err(BrregError {
                    typ: BrregErrorType::InternalServerError,
                    error: Some(format!(
                        "{}: {} {}",
                        brreg_error.trace, brreg_error.error, brreg_error.message
                    )),
                });
            }
            return Err(BrregError {
                typ: BrregErrorType::InternalServerError,
                error: Some("Got unparsable 500 internal server error".to_string()),
            });
        }

        _ => Err(BrregError {
            typ: BrregErrorType::UnexpectedResponse,
            error: Some(format!("Got status code {}", response.status().as_str()).to_string()),
        }),
    }
}

fn get_parent_org(client: &Client, org: &Organization) -> Result<Option<Organization>, BrregError> {
    match org.overordnet_enhet.as_ref() {
        None => Ok(None),
        Some(parent_orgnr) => {
            return Ok(Some(get_organization(client, &parent_orgnr, "enhet")?));
        }
    }
}

fn get_child_orgs(client: &Client, parent_orgnr: &str) -> Result<Option<Underenheter>, BrregError> {
    let http_result = client
        .get("https://data.brreg.no/enhetsregisteret/api/underenheter")
        .query(&[("overordnetEnhet", parent_orgnr)])
        .send();

    if http_result.is_err() {
        return Err(BrregError {
            typ: BrregErrorType::NetworkError,
            error: Some(http_result.unwrap_err().to_string()),
        });
    }
    let mut response = http_result.unwrap();

    let mut body = String::new();
    let read_result = response.read_to_string(&mut body);
    if read_result.is_err() {
        return Err(BrregError {
            typ: BrregErrorType::NetworkError,
            error: Some(read_result.unwrap_err().to_string()),
        });
    }

    match response.status() {
        StatusCode::OK => {
            let json_parse_res = serde_json::from_str::<SearchResponse>(&body);

            if json_parse_res.is_ok() {
                return Ok(json_parse_res.unwrap()._embedded);
            }
            return Err(BrregError {
                typ: BrregErrorType::JsonParseError,
                error: Some(json_parse_res.unwrap_err().to_string()),
            });
        }

        StatusCode::INTERNAL_SERVER_ERROR => {
            let json_parse_res = serde_json::from_str::<BrregInternalServerError>(&body);
            if json_parse_res.is_ok() {
                let brreg_error = json_parse_res.unwrap();

                return Err(BrregError {
                    typ: BrregErrorType::InternalServerError,
                    error: Some(format!(
                        "{}: {} {}",
                        brreg_error.trace, brreg_error.error, brreg_error.message
                    )),
                });
            }
            return Err(BrregError {
                typ: BrregErrorType::JsonParseError,
                error: Some(json_parse_res.unwrap_err().to_string()),
            });
        }

        _ => Err(BrregError {
            typ: BrregErrorType::UnexpectedResponse,
            error: Some(format!("Got status code {}", response.status().as_str()).to_string()),
        }),
    }
}

fn print_address(address: Adresse) {
    // ugh
    for (pos, addresse) in address.adresse.iter().enumerate() {
        if pos != 0 {
            println!("{}  {}", pad_title(""), addresse);
        } else {
            println!("{}", addresse);
        }
    }

    if address.adresse.len() != 0 {
        print!("{}  ", pad_title(""));
    }
    if address.postnummer.is_some() && address.poststed.is_some() {
        println!(
            "{} {}",
            address.postnummer.unwrap_or_default(),
            address.poststed.unwrap_or_default(),
        )
    } else {
        println!(
            "{}{}",
            address.postnummer.unwrap_or_default(),
            address.poststed.unwrap_or_default(),
        )
    }
}

fn pad_title(title: &str) -> String {
    return format!("{:width$}", title, width = MAX_TITLE_LENGTH);
}

fn get_norwegian_bool(input: bool) -> &'static str {
    return if input { "Ja" } else { "Nei" };
}

fn print_org_info(
    org_type: &str,
    org: Organization,
    maybe_parent_org: Option<Organization>,
    maybe_child_orgs: Option<Underenheter>,
) {
    println!("********************** {} **********************", org_type);
    println!("{}: {}", pad_title("Orgnummer"), org.organisasjonsnummer);
    println!("{}: {}", pad_title("Navn"), org.navn);

    if let Some(slettedato) = org.slettedato {
        println!("{}: {}", pad_title("Slettedato"), slettedato);
    }

    if let Some(forretningsadresse) = org.forretningsadresse {
        print!("{}: ", pad_title("Forretningsadresse"));
        print_address(forretningsadresse);
    }
    if let Some(postadresse) = org.postadresse {
        print!("{}: ", pad_title("Postadresse"));
        print_address(postadresse);
    }

    if let Some(hjemmeside) = org.hjemmeside {
        println!("{}: {}", pad_title("Hjemmeside"), hjemmeside);
    }

    if let Some(registreringsdato_enhetsregisteret) = org.registreringsdato_enhetsregisteret {
        println!(
            "{}: {}",
            pad_title("Registrert i Enhetsregisteret"),
            registreringsdato_enhetsregisteret
        );
    }
    if let Some(under_avvikling) = org.under_avvikling {
        println!(
            "{}: {}",
            pad_title("Under avvikling"),
            get_norwegian_bool(under_avvikling)
        );
    }
    if let Some(under_ta_eller_to) = org.under_tvangsavvikling_eller_tvangsopplosning {
        println!(
            "{}: {}",
            pad_title("Under tvangsavvikling eller tvangsoppløsning"),
            get_norwegian_bool(under_ta_eller_to)
        );
    }

    if let Some(parent_org) = maybe_parent_org {
        println!(
            "{}: {} - {}",
            pad_title("Overordnet enhet"),
            parent_org.organisasjonsnummer,
            parent_org.navn
        )
    }

    if let Some(child_orgs) = maybe_child_orgs {
        let underenheter = child_orgs.underenheter;
        if underenheter.len() > 0 {
            print!("{}:", pad_title("Underenheter (20 første)"));
            for (pos, child_org) in underenheter.iter().enumerate() {
                if pos != 0 {
                    println!(
                        "{}  {} - {}",
                        pad_title(""),
                        child_org.organisasjonsnummer,
                        child_org.navn
                    );
                } else {
                    println!(" {} - {}", child_org.organisasjonsnummer, child_org.navn);
                }
            }
        }
    }
}

fn handle_main_error(error: BrregError) {
    // Handles error for the "main" queries, where
    // a 404 or 410 is to be expected, and should
    // be communicated to the user.
    match error.typ {
        BrregErrorType::Gone => {
            eprintln!("Denne enheten er fjernet fra brreg");
        }

        BrregErrorType::NotFound => {
            eprintln!("Fant ikke denne enheten i brreg");
        }

        _ => handle_error(error),
    }

    std::process::exit(1);
}

fn handle_error(error: BrregError) {
    // Handles error for extra queries (child/parent orgs)
    // where it would be wrong to e.g. tell the user the
    // org is missing on 404.
    match error.typ {
        BrregErrorType::NetworkError => {
            eprintln!(
                "Feil under kommunikasjon med brreg: {}",
                error.error.unwrap()
            );
        }

        BrregErrorType::UnexpectedResponse => {
            eprintln!("Uventet svar fra brreg: {}", error.error.unwrap());
        }

        BrregErrorType::InternalServerError => {
            eprintln!("Trøbbel i tårnet hos brreg: {}", error.error.unwrap());
        }

        BrregErrorType::JsonParseError => {
            eprintln!(
                "Klarte ikke lese svaret fra brreg: {}",
                error.error.unwrap()
            );
        }

        _ => {
            panic!("Unexpected error type in handle_error: {:?}", error.typ)
        }
    }

    std::process::exit(1);
}

fn handle_extra_call_to_brreg<T: std::fmt::Debug>(
    result: Result<Option<T>, BrregError>,
) -> Option<T> {
    if result.is_ok() {
        return result.unwrap();
    }
    handle_error(result.unwrap_err());
    panic!("handle_error did not handle error")
}

fn main() {
    let mut args = env::args();
    let cmd_name = args.nth(0).unwrap();
    let args: Vec<String> = args.collect();

    // The user may run it with the org number
    // spread over several arguments, like
    // `brreg-cli 983 544 622`, or with spaces in it,
    // so let's combine all arguments, strip spaces and
    // then see if it's a nine digit number.
    let combined_params = args.join("").replace(" ", "");

    if combined_params == "--version" {
        println!("Version: {} ", VERSION);
        std::process::exit(0);
    }

    if combined_params.len() != 9 || combined_params.parse::<u32>().is_err() {
        eprintln!("Usage: {} *orgnr*", cmd_name);
        eprintln!("(orgnr must be nine numbers)");
        std::process::exit(1);
    }
    let orgnr = combined_params;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .user_agent("https://github.com/magnuswatn/brreg-cli")
        .use_rustls_tls()
        .https_only(true)
        .build()
        .unwrap();

    let main_result = get_organization(&client, &orgnr, "enhet");
    match main_result {
        Ok(org) => {
            let parent_org = handle_extra_call_to_brreg(get_parent_org(&client, &org));
            let child_orgs = handle_extra_call_to_brreg(get_child_orgs(&client, &orgnr));
            print_org_info("Organisasjon", org, parent_org, child_orgs);
        }
        Err(err) => {
            if err.typ == BrregErrorType::NotFound {
                // Maybe an underenhet
                let child_result = get_organization(&client, &orgnr, "underenhet");
                match child_result {
                    Ok(org) => {
                        let parent_org = handle_extra_call_to_brreg(get_parent_org(&client, &org));
                        print_org_info("Underenhet", org, parent_org, None);
                    }
                    Err(child_err) => {
                        handle_main_error(child_err);
                    }
                }
            } else {
                handle_main_error(err);
            }
        }
    }
}
