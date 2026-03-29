"""Tests for the synthesis service."""

from engine.services.synthesis import (
    FORMAT_TEMPLATES,
    OutputFormat,
    SynthesisConfig,
    SynthesisService,
    SynthesisSource,
    SynthesisStatus,
)


class TestSynthesisService:
    def test_create_job(self):
        svc = SynthesisService()
        config = SynthesisConfig(
            job_id="test-001",
            name="Test",
            description="Test dataset",
            source=SynthesisSource.OLLAMA,
            model="llama3.1:8b",
        )
        job = svc.create_job(config)
        assert job.status == SynthesisStatus.QUEUED
        assert job.current_samples == 0
        assert job.config.name == "Test"

    def test_list_jobs(self):
        svc = SynthesisService()
        config = SynthesisConfig(
            job_id="test-002",
            name="Test 2",
            description="Another test",
            source=SynthesisSource.HUGGINGFACE,
            model="google/flan-t5-base",
        )
        svc.create_job(config)
        jobs = svc.list_jobs()
        assert len(jobs) >= 1
        assert any(j["jobId"] == "test-002" for j in jobs)

    def test_job_to_dict(self):
        svc = SynthesisService()
        config = SynthesisConfig(
            job_id="test-003",
            name="Dict Test",
            description="Testing serialization",
            source=SynthesisSource.OLLAMA,
            model="llama3.1:8b",
            target_samples=500,
            output_format=OutputFormat.CSV,
        )
        job = svc.create_job(config)
        d = svc._job_to_dict(job)
        assert d["jobId"] == "test-003"
        assert d["targetSamples"] == 500
        assert d["outputFormat"] == "csv"
        assert d["progress"] == 0

    def test_get_nonexistent_job(self):
        svc = SynthesisService()
        assert svc.get_job("nonexistent") is None


class TestJsonParsing:
    """Test the _parse_json method handles various LLM outputs."""

    def setup_method(self):
        self.svc = SynthesisService()

    def test_clean_json(self):
        result = self.svc._parse_json('{"key": "value"}')
        assert result == {"key": "value"}

    def test_json_with_markdown_fences(self):
        text = '```json\n{"key": "value"}\n```'
        result = self.svc._parse_json(text)
        assert result == {"key": "value"}

    def test_json_with_surrounding_text(self):
        text = 'Here is the output:\n{"key": "value"}\nDone!'
        result = self.svc._parse_json(text)
        assert result == {"key": "value"}

    def test_json_array(self):
        text = '[{"a": 1}, {"b": 2}]'
        result = self.svc._parse_json(text)
        assert result == [{"a": 1}, {"b": 2}]  # valid JSON parses directly

    def test_invalid_json(self):
        result = self.svc._parse_json("not json at all")
        assert result is None

    def test_empty_string(self):
        result = self.svc._parse_json("")
        assert result is None


class TestFormatTemplates:
    def test_all_formats_have_templates(self):
        for fmt in OutputFormat:
            assert fmt in FORMAT_TEMPLATES, f"Missing template for {fmt}"

    def test_templates_have_placeholders(self):
        for fmt, template in FORMAT_TEMPLATES.items():
            assert "{description}" in template
            assert "{sample_num}" in template
