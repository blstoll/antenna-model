#!/usr/bin/env python3
"""
Antenna Model Service - Python API Examples

This script demonstrates how to interact with the Antenna Model Service API using Python.

Requirements:
    pip install requests

Usage:
    python examples/python_examples.py
"""

import json
import requests
from typing import Dict, List, Any


class AntennaModelClient:
    """Python client for the Antenna Model Service API."""

    def __init__(self, base_url: str = "http://localhost:3000"):
        """
        Initialize the client.

        Args:
            base_url: Base URL of the service (default: http://localhost:3000)
        """
        self.base_url = base_url
        self.session = requests.Session()
        self.session.headers.update({"Content-Type": "application/json"})

    def health(self) -> Dict[str, Any]:
        """Check service health (liveness probe)."""
        response = self.session.get(f"{self.base_url}/health")
        response.raise_for_status()
        return response.json()

    def ready(self) -> Dict[str, Any]:
        """Check service readiness."""
        response = self.session.get(f"{self.base_url}/ready")
        response.raise_for_status()
        return response.json()

    def status(self) -> Dict[str, Any]:
        """Get detailed service status."""
        response = self.session.get(f"{self.base_url}/status")
        response.raise_for_status()
        return response.json()

    def list_antennas(self) -> Dict[str, Any]:
        """List all available antennas."""
        response = self.session.get(f"{self.base_url}/api/v1/antennas")
        response.raise_for_status()
        return response.json()

    def get_antenna_details(self, antenna_id: str) -> Dict[str, Any]:
        """Get detailed information about a specific antenna."""
        response = self.session.get(f"{self.base_url}/api/v1/antennas/{antenna_id}")
        response.raise_for_status()
        return response.json()

    def list_antenna_feeds(self, antenna_id: str) -> Dict[str, Any]:
        """List all feeds for a specific antenna."""
        response = self.session.get(f"{self.base_url}/api/v1/antennas/{antenna_id}/feeds")
        response.raise_for_status()
        return response.json()

    def get_feed_details(self, antenna_id: str, feed_id: str) -> Dict[str, Any]:
        """Get detailed information about a specific feed."""
        response = self.session.get(
            f"{self.base_url}/api/v1/antennas/{antenna_id}/feeds/{feed_id}"
        )
        response.raise_for_status()
        return response.json()

    def compute_gain(
        self,
        antenna_id: str,
        feed_id: str,
        vehicle_position: Dict[str, float],
        vehicle_attitude: List[float],
        reflector_boresight: Dict[str, float],
        feed_pointing_location: Dict[str, float],
        emitter_position: Dict[str, float],
        frequency_mhz: float,
        pointing_frequency_mhz: float = None,
        include_reference: bool = False,
    ) -> Dict[str, Any]:
        """
        Compute antenna gain.

        Args:
            antenna_id: Antenna identifier
            feed_id: Feed identifier
            vehicle_position: Vehicle position dict with x, y, z
            vehicle_attitude: Normalized quaternion as a [w, x, y, z] list
            reflector_boresight: Reflector boresight position dict with x, y, z
            feed_pointing_location: Earth location the beam is aimed at, dict with x, y, z
            emitter_position: Emitter position dict with x, y, z
            frequency_mhz: Operating frequency in MHz
            pointing_frequency_mhz: Optional pointing frequency for beam squint
            include_reference: Whether to include reference gain computation

        Returns:
            Gain response dictionary
        """
        request_data = {
            "antenna_id": antenna_id,
            "feed_id": feed_id,
            "vehicle_position": vehicle_position,
            "vehicle_attitude": vehicle_attitude,
            "reflector_boresight": reflector_boresight,
            "feed_pointing_location": feed_pointing_location,
            "emitter_position": emitter_position,
            "frequency_mhz": frequency_mhz,
            "include_reference": include_reference,
        }

        if pointing_frequency_mhz is not None:
            request_data["pointing_frequency_mhz"] = pointing_frequency_mhz

        response = self.session.post(
            f"{self.base_url}/api/v1/gain", json=request_data
        )
        response.raise_for_status()
        return response.json()

    def compute_gain_batch(self, evaluations: List[Dict[str, Any]]) -> Dict[str, Any]:
        """
        Compute gain for multiple requests in batch.

        Args:
            evaluations: List of gain request dictionaries

        Returns:
            Batch response dictionary with results list
        """
        request_data = {"evaluations": evaluations}
        response = self.session.post(
            f"{self.base_url}/api/v1/gain/batch", json=request_data
        )
        response.raise_for_status()
        return response.json()

    def generate_heatmap(
        self,
        antenna_id: str,
        feed_id: str,
        vehicle_position: Dict[str, float],
        reflector_boresight: Dict[str, float],
        feed_pointing_location: Dict[str, float],
        frequency_mhz: float,
        grid_config: Dict[str, Any],
        pointing_frequency_mhz: float = None,
    ) -> Dict[str, Any]:
        """
        Generate a loss heatmap.

        Args:
            antenna_id: Antenna identifier
            feed_id: Feed identifier
            vehicle_position: Vehicle position dict with x, y, z
            reflector_boresight: Reflector boresight position dict
            feed_pointing_location: Earth location the beam is aimed at, dict with x, y, z
            frequency_mhz: Operating frequency in MHz
            grid_config: Grid configuration dict
            pointing_frequency_mhz: Optional pointing frequency

        Returns:
            Heatmap response dictionary
        """
        request_data = {
            "antenna_id": antenna_id,
            "feed_id": feed_id,
            "vehicle_position": vehicle_position,
            "reflector_boresight": reflector_boresight,
            "feed_pointing_location": feed_pointing_location,
            "frequency_mhz": frequency_mhz,
            "grid_config": grid_config,
        }

        if pointing_frequency_mhz is not None:
            request_data["pointing_frequency_mhz"] = pointing_frequency_mhz

        response = self.session.post(
            f"{self.base_url}/api/v1/heatmap", json=request_data
        )
        response.raise_for_status()
        return response.json()


def format_gain(result: dict) -> str:
    """Render one evaluation's gain, including the failed case.

    `gain_db` is serialized as JSON **null** when the evaluation failed (the
    service's `nan_as_null` convention), so formatting it with `:.2f`
    unconditionally raises `TypeError: unsupported format string passed to
    NoneType.__format__`. That is not a hypothetical: `/api/v1/gain/batch`
    returns HTTP 200 with failed items, so `raise_for_status()` does not catch
    it and the traceback comes from the print, not the request.

    A failed item carries a typed `error` instead; report that.
    """
    gain = result.get("gain_db")
    if gain is None:
        error = result.get("error") or {}
        code = error.get("code", "unknown")
        message = error.get("message", "no gain returned")
        return f"FAILED [{code}] {message}"
    return f"{gain:.2f} dB"


def main():
    """Run example API calls."""
    client = AntennaModelClient()

    print("=" * 60)
    print("Antenna Model Service - Python Examples")
    print("=" * 60)

    # 1. Check service health
    print("\n1. Health Check")
    try:
        health = client.health()
        print(f"   Status: {health['status']}")
    except requests.exceptions.RequestException as e:
        print(f"   Error: Service not running. Start with: cargo run --release --bin antenna-model")
        return

    # 2. Get service status
    print("\n2. Service Status")
    status = client.status()
    print(f"   Version: {status['version']}")
    print(f"   Uptime: {status['uptime_seconds']} seconds")
    print(f"   Antennas loaded: {status.get('antenna_count', 0)}")

    # 3. List all antennas
    print("\n3. List Antennas")
    antennas = client.list_antennas()
    for antenna in antennas.get("antennas", []):
        print(f"   - {antenna['id']}: {antenna['name']} ({antenna['feed_count']} feeds)")

    # 4. Get antenna details (if any antennas are loaded)
    if antennas.get("antennas"):
        antenna_id = antennas["antennas"][0]["id"]
        print(f"\n4. Antenna Details: {antenna_id}")
        details = client.get_antenna_details(antenna_id)
        print(f"   Name: {details['name']}")
        print(f"   Diameter: {details['physical_parameters']['diameter_m']} m")
        print(f"   Feeds: {', '.join([f['id'] for f in details['feeds']])}")

        # 5. Single gain computation example
        if details["feeds"]:
            feed_id = details["feeds"][0]["id"]
            print(f"\n5. Gain Computation: {antenna_id}/{feed_id}")

            try:
                result = client.compute_gain(
                    antenna_id=antenna_id,
                    feed_id=feed_id,
                    vehicle_position={"x": 6500000.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
                    vehicle_attitude=[0.5, 0.5, 0.5, 0.5],  # body +Z -> ECEF +X (the boresight); identity would be degenerate here
                    reflector_boresight={"x": 6500010.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
                    feed_pointing_location={"x": 6500005.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
                    emitter_position={"x": 7000000.0, "y": 0.0, "z": 500000.0, "coordinate_system": "ecef"},
                    frequency_mhz=8450.0,
                    include_reference=True,
                )
                print(f"   Gain: {format_gain(result)}")
                if result.get("reference_gain_db") is not None:
                    print(f"   Reference Gain: {result['reference_gain_db']:.2f} dB")
                if result.get("loss_db") is not None:
                    print(f"   Loss: {result['loss_db']:.2f} dB")
                print(f"   Computation time: {result['metadata']['computation_time_ms']:.1f} ms")
            except requests.exceptions.HTTPError as e:
                print(f"   Error: {e}")

            # 6. Batch computation example
            print(f"\n6. Batch Gain Computation (3 evaluations)")
            try:
                batch_result = client.compute_gain_batch(
                    evaluations=[
                        {
                            "antenna_id": antenna_id,
                            "feed_id": feed_id,
                            "vehicle_position": {"x": 6500000.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
                            "vehicle_attitude": [0.5, 0.5, 0.5, 0.5],
                            "reflector_boresight": {"x": 6500010.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
                            "feed_pointing_location": {"x": 6500005.0, "y": 0.0, "z": 0.0, "coordinate_system": "ecef"},
                            "emitter_position": {"x": 7000000.0, "y": y, "z": 500000.0, "coordinate_system": "ecef"},
                            "frequency_mhz": 8450.0,
                            "include_reference": False,
                        }
                        for y in [0.0, 250000.0, 500000.0]
                    ]
                )
                meta = batch_result["metadata"]
                total_evals = meta.get("count", len(batch_result["results"]))
                print(f"   Total evaluations: {total_evals}")
                print(f"   Total time: {meta['total_computation_time_ms']:.1f} ms")
                # /gain/batch answers HTTP 200 even when items fail, so the status
                # code raise_for_status() checks is NOT the success signal —
                # failure_count is. A failed item carries gain_db: null (the
                # nan_as_null convention) plus a typed `error`.
                print(f"   Failures: {meta.get('failure_count', 0)}")
                for i, result in enumerate(batch_result["results"]):
                    print(f"   Result {i+1}: {format_gain(result)}")
            except requests.exceptions.HTTPError as e:
                print(f"   Error: {e}")

    print("\n" + "=" * 60)
    print("Examples completed!")
    print("=" * 60)


if __name__ == "__main__":
    main()
