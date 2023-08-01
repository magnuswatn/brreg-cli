use reqwest::{blocking::Client, StatusCode};
use serde_derive::Deserialize;
use std::io::Read;
use std::time::Duration;
use std::{env, fmt};

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
#[derive(Deserialize, Debug, PartialEq)]
struct RemovedOrganization {
    organisasjonsnummer: String,
    slettedato: String,
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

struct OrganizationWithRelatedOrgs {
    org_type: &'static str,
    org: Organization,
    maybe_parent_org: Option<Organization>,
    maybe_child_orgs: Option<Underenheter>,
}

struct RemovedOrganizationWithRelatedOrgs {
    org_type: &'static str,
    org: RemovedOrganization,
}

enum BrregOrgNrSearchResult {
    Found(OrganizationWithRelatedOrgs),
    NotFound(),
    Removed(RemovedOrganizationWithRelatedOrgs),
}

// https://data.brreg.no/enhetsregisteret/api/docs/index.html#_500_feil_p%C3%A5_server
#[derive(Deserialize, Debug)]
struct BrregInternalServerError {
    trace: String,
    error: String,
    message: String,
}
impl fmt::Display for BrregInternalServerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {} {}", self.trace, self.error, self.message)
    }
}

#[derive(Debug, PartialEq)]
enum BrregError {
    NotFound,
    Gone(RemovedOrganization),
    InternalServerError(String),
    NetworkError(String),
    UnexpectedResponse(String),
    JsonParseError(String),
}

impl From<reqwest::Error> for BrregError {
    fn from(error: reqwest::Error) -> Self {
        BrregError::NetworkError(error.to_string())
    }
}
impl From<std::io::Error> for BrregError {
    fn from(error: std::io::Error) -> Self {
        BrregError::JsonParseError(error.to_string())
    }
}
impl From<serde_json::Error> for BrregError {
    fn from(error: serde_json::Error) -> Self {
        BrregError::JsonParseError(error.to_string())
    }
}

fn get_organization(client: &Client, orgnr: &str, typ: &str) -> Result<Organization, BrregError> {
    let mut response = client
        .get(format!(
            "https://data.brreg.no/enhetsregisteret/api/{}er/{}",
            typ, orgnr
        ))
        .send()?;

    let mut body = String::new();
    response.read_to_string(&mut body)?;

    match response.status() {
        StatusCode::OK => Ok(serde_json::from_str::<Organization>(&body)?),

        StatusCode::NOT_FOUND => Err(BrregError::NotFound),

        StatusCode::GONE => Err(BrregError::Gone(
            serde_json::from_str::<RemovedOrganization>(&body)?,
        )),

        StatusCode::INTERNAL_SERVER_ERROR => {
            let json_parse_res = serde_json::from_str::<BrregInternalServerError>(&body);

            match json_parse_res {
                Ok(brreg_error) => Err(BrregError::InternalServerError(brreg_error.to_string())),
                Err(_) => Err(BrregError::InternalServerError(
                    "Got unparsable 500 internal server error".to_string(),
                )),
            }
        }

        _ => Err(BrregError::UnexpectedResponse(
            format!("Got status code {}", response.status().as_str()).to_string(),
        )),
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
    let mut response = client
        .get("https://data.brreg.no/enhetsregisteret/api/underenheter")
        .query(&[("overordnetEnhet", parent_orgnr)])
        .send()?;

    let mut body = String::new();
    response.read_to_string(&mut body)?;

    match response.status() {
        StatusCode::OK => Ok(serde_json::from_str::<SearchResponse>(&body)?._embedded),

        StatusCode::INTERNAL_SERVER_ERROR => {
            let json_parse_res = serde_json::from_str::<BrregInternalServerError>(&body);

            match json_parse_res {
                Ok(brreg_error) => Err(BrregError::InternalServerError(brreg_error.to_string())),
                Err(parse_error) => Err(BrregError::InternalServerError(parse_error.to_string())),
            }
        }

        _ => Err(BrregError::UnexpectedResponse(
            format!("Got status code {}", response.status().as_str()).to_string(),
        )),
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

fn print_org_info(org_with_related_orgs: OrganizationWithRelatedOrgs) {
    let org = org_with_related_orgs.org;
    println!(
        "********************** {} **********************",
        org_with_related_orgs.org_type
    );
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

    if let Some(parent_org) = org_with_related_orgs.maybe_parent_org {
        println!(
            "{}: {} - {}",
            pad_title("Overordnet enhet"),
            parent_org.organisasjonsnummer,
            parent_org.navn
        )
    }

    if let Some(child_orgs) = org_with_related_orgs.maybe_child_orgs {
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

fn search_org_by_orgnr(client: &Client, orgnr: &str) -> Result<BrregOrgNrSearchResult, BrregError> {
    let main_result = get_organization(&client, &orgnr, "enhet");
    match main_result {
        Ok(org) => {
            let maybe_parent_org = get_parent_org(&client, &org)?;
            let maybe_child_orgs = get_child_orgs(&client, &orgnr)?;
            Ok(BrregOrgNrSearchResult::Found(OrganizationWithRelatedOrgs {
                org_type: "Organisasjon",
                org,
                maybe_parent_org,
                maybe_child_orgs,
            }))
        }
        Err(err) => {
            match err {
                BrregError::NotFound => {
                    // Maybe an underenhet
                    let child_result = get_organization(&client, &orgnr, "underenhet");
                    match child_result {
                        Ok(org) => {
                            let maybe_parent_org = get_parent_org(&client, &org)?;

                            Ok(BrregOrgNrSearchResult::Found(OrganizationWithRelatedOrgs {
                                org_type: "Underenhet",
                                org,
                                maybe_parent_org,
                                maybe_child_orgs: None,
                            }))
                        }
                        Err(child_err) => match child_err {
                            BrregError::NotFound => {
                                // Not found as parent nor as child
                                Ok(BrregOrgNrSearchResult::NotFound())
                            }
                            BrregError::Gone(removed_org) => Ok(BrregOrgNrSearchResult::Removed(
                                RemovedOrganizationWithRelatedOrgs {
                                    org_type: "underenhet",
                                    org: removed_org,
                                },
                            )),
                            _ => Err(child_err),
                        },
                    }
                }
                BrregError::Gone(removed_org) => Ok(BrregOrgNrSearchResult::Removed(
                    RemovedOrganizationWithRelatedOrgs {
                        org_type: "organisasjon",
                        org: removed_org,
                    },
                )),
                _ => Err(err),
            }
        }
    }
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
        std::process::exit(64);
    }
    let orgnr = combined_params;

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .user_agent("https://github.com/magnuswatn/brreg-cli")
        .use_rustls_tls()
        .https_only(true)
        .build()
        .unwrap();

    let search_result = search_org_by_orgnr(&client, &orgnr);

    match search_result {
        Ok(result) => match result {
            BrregOrgNrSearchResult::Found(org) => print_org_info(org),
            BrregOrgNrSearchResult::NotFound() => {
                eprintln!("Fant ikke denne enheten i brreg");
                std::process::exit(90);
            }
            BrregOrgNrSearchResult::Removed(removed_org) => {
                eprintln!(
                    "Denne {}en ble fjernet fra brreg {}",
                    removed_org.org_type, removed_org.org.slettedato
                );
                std::process::exit(91);
            }
        },
        Err(error) => {
            match error {
                BrregError::NetworkError(err) => {
                    eprintln!("Feil under kommunikasjon med brreg: {}", err);
                }

                BrregError::UnexpectedResponse(err) => {
                    eprintln!("Uventet svar fra brreg: {}", err);
                }

                BrregError::InternalServerError(err) => {
                    eprintln!("Trøbbel i tårnet hos brreg: {}", err);
                }

                BrregError::JsonParseError(err) => {
                    eprintln!("Klarte ikke lese svaret fra brreg: {}", err);
                }

                // This can only happen when we look up related orgs,
                // and it's funky if referenced org is missing
                BrregError::Gone(_) | BrregError::NotFound => {
                    eprintln!("En referert enhet manglet i brreg, dette var ikke forventet");
                }
            }

            std::process::exit(1);
        }
    }
}
