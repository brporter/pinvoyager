import unittest

from generate_association import association


class AssociationTests(unittest.TestCase):
    def test_association_names_the_signed_app_and_supported_web_routes(self):
        result = association("ABCDE12345.com.pinvoyager.app")
        details = result["applinks"]["details"]
        self.assertEqual(details[0]["appIDs"], ["ABCDE12345.com.pinvoyager.app"])
        self.assertEqual({item["/"] for item in details[0]["components"]},
                         {"/voyages/*", "/u/*", "/upload", "/settings"})
        self.assertNotIn("auth", str(result))

    def test_unexpanded_or_unrelated_application_identifiers_are_rejected(self):
        for value in ("", "TEAM_ID.com.pinvoyager.app", "abcde12345.com.pinvoyager.app",
                      "ABCDE12345.com.example.app", "ABCDE12345.com.pinvoyager.app.extra"):
            with self.subTest(value=value), self.assertRaises(ValueError):
                association(value)


if __name__ == "__main__":
    unittest.main()
