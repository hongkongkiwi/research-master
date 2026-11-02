# Research Sources

Research Master MCP supports **36 academic research sources** (32 in the default build) with different pricing models, API requirements, and capabilities.

## Sources Overview

| Source | Search | Download | Read* | Citations | DOI Lookup | Author Search |
|--------|--------|----------|-------|-----------|------------|---------------|
| [arXiv](https://arxiv.org) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| [Semantic Scholar](https://semanticscholar.org) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| [OpenAlex](https://openalex.org) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| [ORCID](https://orcid.org) | ✅ | ❌ | ❌ | ❌ | ❌ | ✅ |
| [PubMed](https://pubmed.ncbi.nlm.nih.gov) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| [PMC](https://www.ncbi.nlm.nih.gov/pmc) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| [bioRxiv](https://biorxiv.org) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| [medRxiv](https://medrxiv.org) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| [ClinicalTrials.gov](https://clinicaltrials.gov) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [HAL](https://hal.science) | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| [DBLP](https://dblp.org) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| [CiteSeerX](https://citeseerx.ist.psu.edu) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [CrossRef](https://www.crossref.org) | ❌ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [IACR ePrint](https://eprint.iacr.org) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| [SSRN](https://www.ssrn.com) | ✅ | ✅ | ✅ | ❌ | ❌ | ❌ |
| [CORE](https://core.ac.uk) | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| [EuropePMC](https://europepmc.org) | ✅ | ❌ | ✅ | ❌ | ❌ | ❌ |
| [Dimensions](https://app.dimensions.ai) | ✅ | ❌ | ❌ | ✅ | ❌ | ❌ |
| [IEEE Xplore](https://ieeexplore.ieee.org) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| [Zenodo](https://zenodo.org) | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| [OpenAIRE](https://openaire.eu) | ✅ | ✅ | ❌ | ❌ | ✅ | ❌ |
| [DataCite](https://datacite.org) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [INSPIRE-HEP](https://inspirehep.net) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [NASA ADS](https://ui.adsabs.harvard.edu) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [Unpaywall](https://unpaywall.org) | ❌ | ✅ | ❌ | ❌ | ✅ | ❌ |
| [MDPI](https://mdpi.com) | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| [JSTOR](https://www.jstor.org) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| [SciSpace](https://scispace.net) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| [ACM DL](https://dl.acm.org) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [Connected Papers](https://www.connectedpapers.com) | ✅ | ❌ | ❌ | ✅ | ❌ | ❌ |
| [DOAJ](https://doaj.org) | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| [WorldWideScience](https://www.worldwidescience.org) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |
| [OSF Preprints](https://osf.io/preprints) | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| [BASE](https://www.base-search.net) | ✅ | ✅ | ✅ | ❌ | ✅ | ❌ |
| [Springer](https://link.springer.com) | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ |
| [Google Scholar](https://scholar.google.com) | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ |

*PDF text extraction requires poppler/libpoppler to be installed on your system.

## Source Pricing and API Requirements

### Source Comparison Table

| Source | Pricing | API Key Required | Free Tier | Rate Limits (no key) | Notes |
|--------|---------|------------------|-----------|----------------------|-------|
| [arXiv](https://arxiv.org) | Free | No | Unlimited | 1 request/sec | Open access preprints |
| [Semantic Scholar](https://semanticscholar.org) | Freemium | Optional | Yes | 1 request/sec | 100 papers/day free; API key increases limits |
| [OpenAlex](https://openalex.org) | Free | Optional | Unlimited | 10 requests/sec | Email recommended for polite pool |
| [PubMed](https://pubmed.ncbi.nlm.nih.gov) | Free | No | Unlimited | 3 requests/sec | NIH funded, no API key needed |
| [PMC](https://www.ncbi.nlm.nih.gov/pmc) | Free | No | Unlimited | 3 requests/sec | Full-text biomedical literature |
| [bioRxiv](https://biorxiv.org) | Free | No | Unlimited | 1 request/sec | Biology preprints |
| [medRxiv](https://medrxiv.org) | Free | No | Unlimited | Global default | Health sciences preprints |
| [ORCID](https://orcid.org) | Free | No | Unlimited | 2 requests/sec | Researcher profiles and works |
| [HAL](https://hal.science) | Free | No | Unlimited | 1 request/sec | French open access repository |
| [DBLP](https://dblp.org) | Free | No | Unlimited | 1 request/sec | Computer science bibliography only |
| [CiteSeerX](https://citeseerx.ist.psu.edu) | Free | No | Unlimited | Global default | Computer science paper search |
| [CrossRef](https://www.crossref.org) | Free | No | Unlimited | 1 request/sec | DOI lookup only, no full search |
| [IACR ePrint](https://eprint.iacr.org) | Free | No | Unlimited | 1 request/sec | Cryptology research only |
| [SSRN](https://www.ssrn.com) | Free | No | Limited | 1 request/sec | Social sciences preprints |
| [CORE](https://core.ac.uk) | Freemium | Optional | Yes | 1 request/sec | Aggregates open access papers |
| [EuropePMC](https://europepmc.org) | Free | No | Unlimited | 1 request/sec | Biomedical literature, indexes PubMed/PMC |
| [Dimensions](https://app.dimensions.ai) | Free | No | Unlimited | 1 request/sec | Research metrics and discovery |
| [IEEE Xplore](https://ieeexplore.ieee.org) | Paid | Required* | Yes | 3 requests/sec | Free API key from developer.ieee.org |
| [Zenodo](https://zenodo.org) | Free | No | Unlimited | 1 request/sec | CERN-hosted open science repository |
| [OpenAIRE](https://openaire.eu) | Free | No | Unlimited | Global default | EU open access research aggregator |
| [ClinicalTrials.gov](https://clinicaltrials.gov) | Free | No | Unlimited | Global default | NIH clinical study registry (API v2) |
| [DataCite](https://datacite.org) | Free | No | Unlimited | 1 request/sec | DOI metadata registry; set `RESEARCH_MASTER_USER_AGENT` with contact address |
| [INSPIRE-HEP](https://inspirehep.net) | Free | No | Unlimited | 1 request/sec | High-energy physics literature database |
| [NASA ADS](https://ui.adsabs.harvard.edu) | Free | Required* | Yes | ~5 requests/sec | Astronomy and physics; token from ui.adsabs.harvard.edu (`ADS_API_KEY`) |
| [Unpaywall](https://unpaywall.org) | Free | No | Unlimited | 1 request/sec | Find open access versions of papers |
| [MDPI](https://mdpi.com) | Free | No | Unlimited | 1 request/sec | Open access publisher |
| [JSTOR](https://www.jstor.org) | Paid | Required* | Limited | 1 request/sec | Historical journals, requires institutional access |
| [SciSpace](https://scispace.net) | Free | No | Unlimited | 1 request/sec | Not in default builds: public endpoint no longer resolves |
| [ACM DL](https://dl.acm.org) | Paid | Required* | Limited | 3 requests/sec | Computer science, free API key available |
| [Connected Papers](https://www.connectedpapers.com) | Free | No | Unlimited | 1 request/sec | Not in default builds: no public REST API |
| [DOAJ](https://doaj.org) | Free | No | Unlimited | 1 request/sec | Quality open access journals |
| [WorldWideScience](https://www.worldwidescience.org) | Free | No | Unlimited | 1 request/sec | Not in default builds: service retired, host no longer resolves |
| [OSF Preprints](https://osf.io/preprints) | Free | No | Unlimited | 1 request/sec | Open Science Framework |
| [BASE](https://www.base-search.net) | Free | No | Unlimited | 1 request/sec | Bielefeld Academic Search Engine |
| [Springer](https://link.springer.com) | Paid | Required* | Limited | 1 request/sec | Academic publisher, some open access |
| [Google Scholar](https://scholar.google.com) | Free | No | Limited | 1 request/sec | Disabled by default, scraping-based |

*Requires API key for full access. Free API keys are available but have rate limits.

## Detailed Source Information

### Completely Free Sources (No API Key Needed)

The following sources are completely free to use without any API key requirements.

**arXiv** is the premier open access preprint repository for physics, mathematics, computer science, and related fields. Operated by Cornell University, it provides unlimited access to millions of preprints without any API key or registration requirements.

**PubMed** and **PMC** are NIH-funded databases providing free access to biomedical and life sciences literature. PubMed contains abstracts and citations, while PMC provides full-text articles. Both are completely free and require no API key. PubMed results carry `paper_type` from the article's publication types (e.g. `Journal Article;Review`), and `search_papers` accepts a `filters.mesh` value with semicolon-separated MeSH clauses: `Term` explodes to all narrower headings, `Term:noexp` restricts to the exact heading, `Term/subheading` qualifies the heading with a subheading, and a clause that already contains a bracket passes through verbatim.

**ClinicalTrials.gov** is the NIH registry of clinical studies, queried through its free API v2 without a key. It supports search; study records have no DOI of their own, so a DOI query (`source: "clinical_trials"`) returns the first study citing that DOI rather than the DOI's own paper, and is deliberately excluded from the cross-source `lookup_by_doi` fan-out for that reason. Paper IDs are NCT numbers (e.g. `NCT03850691`, case-insensitive) and results carry `paper_type` `clinical trial` with the study's overall status in `status`.

**OpenAlex** is an open infrastructure funded by various academic organizations, providing a comprehensive catalog of scholarly papers, authors, institutions, and more. List and search endpoints are metered: keyless calls share a small free daily budget per IP address, while a free API key (`OPENALEX_API_KEY`, created at openalex.org/settings/api) gets its own larger daily budget. The retired mailto "polite pool" is gone; `OPENALEX_EMAIL` now only identifies your requests. Results carry `paper_type` from the work's `type` verbatim, `categories` from topic display names, and a `venue` key in `extra` with the publishing source's display name; retracted works are flagged with `is_retracted: true`.

**HAL** is France's national open access repository, providing free access to academic papers from French institutions. It requires no API key and offers unlimited queries.

**DBLP** specializes in computer science bibliography, providing free access to citations for conference proceedings, journals, and books in CS. While comprehensive for its niche, it only supports search and does not provide full paper downloads.

**CrossRef** provides DOI (Digital Object Identifier) lookup services for free. It does not support full-text search but is invaluable for looking up paper metadata by DOI.

**IACR ePrint** focuses exclusively on cryptology research from the International Association for Cryptologic Research. All papers are freely available, and no API key is required.

**bioRxiv** is the primary preprint server for biology and life sciences, operated by Cold Spring Harbor Laboratory. All preprints are free to access.

**Zenodo** is CERN-hosted open science repository, providing free access to research data and papers from researchers worldwide. No API key is required for basic access.

**Unpaywall** helps find open access versions of papers by checking thousands of open access repositories. It's completely free and requires no API key.

**DOAJ** (Directory of Open Access Journals) indexes only peer-reviewed, open access journals. It provides free access to journal metadata and paper information.

**Connected Papers** helps researchers discover related papers using a visual graph approach. It offers a free tier with reasonable rate limits.

**WorldWideScience** is a global science gateway providing access to scientific databases from multiple countries. It's completely free and operated by the scientific community.

**OSF Preprints** provides access to preprints from the Open Science Framework, covering many disciplines. No API key is required. A `filters.provider` value on `search_papers` restricts results to one hosted preprint provider by slug (e.g. `psyarxiv`, `socarxiv`, `engrxiv`, `eartharxiv`, `edarxiv`, `mindrxiv`, `mediarxiv`, `lawarxiv`; 32 providers total). All OSF results carry `paper_type` `preprint`.

**BASE** (Bielefeld Academic Search Engine) aggregates content from over 4,000 data sources. It provides free access to academic resources.

**Dimensions** is a research discovery platform funded by Digital Science, offering free access to papers, grants, patents, and clinical trials.

**MDPI** is an open access publisher that provides free access to all its journal articles. The website is free to use.

**Google Scholar** is completely free but disabled by default because it uses scraping techniques rather than an official API. See [Google Scholar Setup](#google-scholar-setup) for how to enable it.

### Freemium Sources (Optional API Key for Higher Limits)

**Semantic Scholar** offers a free tier with approximately 100 papers per day and 1 request per second. Providing a `SEMANTIC_SCHOLAR_API_KEY` significantly increases these limits. The API key is free to obtain from the Semantic Scholar website.

**CORE** aggregates millions of open access papers from repositories worldwide. A free API key is available at core.ac.uk and increases rate limits. Without a key, functionality is limited.

**SciSpace** provides a research discovery platform with both free and paid tiers. An optional API key can be configured for enhanced access.

### API Key Required (Free Registration Available)

**IEEE Xplore** requires an API key for programmatic access. Free API keys are available by registering at developer.ieee.org. The free tier provides limited requests per second, and full access requires institutional subscriptions.

**ACM Digital Library** requires an API key for its programmatic search API. Free API keys can be obtained from the ACM Developer Portal (developers.acm.org). Without an API key, functionality is severely limited.

**Springer** may require an API key for full programmatic access. Check the Springer Nature Developer Portal for registration and access details.

**JSTOR** is primarily a subscription-based service. Institutional access is typically required, and API access may need additional licensing.

## Source-Specific Configuration

### Google Scholar Setup

Google Scholar is disabled by default both at compile-time and runtime due to its scraping-based approach:

1. **Compile with the feature flag:**
   ```bash
   cargo build --features google_scholar
   ```

2. **Enable at runtime:**
   ```bash
   export GOOGLE_SCHOLAR_ENABLED=true
   ```

### Semantic Scholar API Key

```bash
export SEMANTIC_SCHOLAR_API_KEY="your-api-key-here"
```

Get your free API key at: https://www.semanticscholar.org/product/api

### CORE API Key

```bash
export CORE_API_KEY="your-core-api-key"
```

Register for a free API key at: https://core.ac.uk/services/api

### IEEE Xplore API Key

```bash
export IEEE_XPLORE_API_KEY="your-ieee-api-key"
```

Register for a free API key at: https://developer.ieee.org/

### ACM API Key

```bash
export ACM_API_KEY="your-acm-api-key"
```

Register for a free API key at: https://developers.acm.org/

### OpenAlex Email (Recommended)

```bash
export OPENALEX_EMAIL="your-email@example.com"
```

Providing an email gives you access to OpenAlex's "polite pool" with better rate limits.

## Rate Limit Configuration

Without API keys, sources enforce rate limits to prevent abuse. You can customize these limits:

```bash
# Semantic Scholar rate limit (requests per second)
export SEMANTIC_SCHOLAR_RATE_LIMIT=5

# Global rate limit
export RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND=10

# Disable rate limiting entirely
export RESEARCH_MASTER_RATE_LIMITS_DEFAULT_REQUESTS_PER_SECOND=0
```

**Note:** Disabling rate limits or setting them too high may result in your IP being blocked by the source APIs. Use with caution.

## Advanced Search Capabilities

- **Keyword search** with relevance ranking
- **Year range filtering**: `"2020"`, `"2018-2022"`, `"2010-"`, `"-2015"`
- **Category/subject filtering** for categorized sources
- **Type filtering**: `include_types`/`exclude_types` tokens matched case-insensitively against reported types and categories, with `strict_types` for strict behavior
- **Source-specific filters**: PubMed MeSH clauses (`filters.mesh`) and OSF provider slugs (`filters.provider`)
- **Author-specific search** (supported sources)
- **DOI-based lookup** across multiple sources

## PDF Management

- Download papers directly to your local filesystem
- Automatic directory creation
- Configurable download paths
- Optional organization by source
- **PDF text extraction** for reading paper contents (requires poppler)

## Citation Analysis

- **Forward citations**: Find papers that cite a given paper
- **References**: Discover papers referenced by a given paper
- **Related papers**: Explore similar research
- **Citation count tracking**

## Deduplication

Intelligent duplicate detection across sources:
- DOI matching (exact)
- Title similarity (Jaro-Winkler algorithm, 0.95+ threshold)
- Author verification
- Multiple strategies: keep first, keep last, or mark duplicates

## Intelligent Source Management

- **Auto-disable sources** that fail to initialize (e.g., missing API keys)
- **Enable specific sources** via environment variable
- Graceful degradation when some sources are unavailable
- **Google Scholar** is disabled by default and requires `GOOGLE_SCHOLAR_ENABLED=true` to activate

## Related Documentation

- [Installation](installation.md) - How to install Research Master MCP
- [Usage](usage.md) - CLI commands and options
- [Configuration](configuration.md) - Environment variables and config file
- [Development](development.md) - Adding new sources
