resource "aws_db_instance" "unsafe_public_fixture" {
  publicly_accessible = true
}